//! Page geometry, table hit testing, selection and navigation.
use super::*;

impl Inner {
    /// Past the first or last line of a table cell, Hancom moves to the cell above or below
    /// in the caret's column, and out of the table past its first or last row: the target
    /// line on this page and, inside the table, the x to place the caret at.
    fn across_cells(
        idx: &PageIndex,
        paths: &std::collections::HashMap<usize, hm_edit::ListPath>,
        list: &hm_edit::ListPath,
        x: i32,
        up: bool,
    ) -> Option<(usize, Option<i32>)> {
        let (last, host) = list.steps.split_last()?;
        let n = host.len();
        let cells: Vec<_> = idx
            .cells
            .iter()
            .filter_map(|b| paths.get(&b.cell).map(|l| (b, l)))
            .filter(|(_, l)| {
                l.section == list.section
                    && l.steps.len() == n + 1
                    && l.steps[..n] == *host
                    && (l.steps[n].para, l.steps[n].control) == (last.para, last.control)
            })
            .collect();
        let line_y = idx.lines.iter().filter(|l| l.list == *list).map(|l| l.y);
        let line_y = if up { line_y.min() } else { line_y.max() }?;
        let (here, _) = cells
            .iter()
            .filter(|(_, l)| *l == list)
            .min_by_key(|(b, _)| (line_y < b.y || line_y >= b.y + b.height, b.y))?;
        let inside = |b: &hm_render::CellBox, x: i32| x.clamp(b.x, b.x + b.width - 1);
        let xc = inside(here, x);
        // Touching cells share an edge; a few HU absorb rounding.
        const SLACK: i32 = 10;
        let next = cells
            .iter()
            .filter(|(b, _)| b.x <= xc && xc < b.x + b.width)
            .filter(|(b, _)| {
                if up {
                    b.y + b.height <= here.y + SLACK && b.y < here.y
                } else {
                    b.y + SLACK >= here.y + here.height && b.y > here.y
                }
            })
            .min_by_key(|(b, _)| {
                if up {
                    here.y - (b.y + b.height)
                } else {
                    b.y - (here.y + here.height)
                }
            });
        if let Some((b, cell)) = next {
            let line = idx
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.list == **cell)
                .min_by_key(|(_, l)| if up { -(l.y + l.height) } else { l.y })?;
            return Some((line.0, Some(inside(b, xc))));
        }
        // Past the table's first or last row: the nearest line of the table's own list.
        let top = cells.iter().map(|(b, _)| b.y).min()?;
        let bottom = cells.iter().map(|(b, _)| b.y + b.height).max()?;
        let outer = hm_edit::ListPath {
            section: list.section,
            steps: host.to_vec(),
        };
        idx.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                l.list == outer
                    && if up {
                        l.y + l.height <= top
                    } else {
                        l.y >= bottom
                    }
            })
            .min_by_key(|(_, l)| {
                let dy = if up {
                    top - (l.y + l.height)
                } else {
                    l.y - bottom
                };
                let dx = if x < l.left() {
                    l.left() - x
                } else {
                    (x - l.right()).max(0)
                };
                (dy, dx)
            })
            .map(|(i, _)| (i, None))
    }

    /// The body's floating objects and tables on page `i`, bottom to top.
    pub(super) fn objects(&self, i: usize) -> Vec<ObjectTarget> {
        use hm_layout::objects::Layer;
        let Some(page) = self.layout.pages.get(i) else {
            return Vec::new();
        };
        let section = page.section;
        let Some(paras) = self
            .editor
            .doc
            .sections
            .get(section as usize)
            .map(|s| &s.paras)
        else {
            return Vec::new();
        };
        let control = |para: u32, k: u32| paras.get(para as usize)?.controls.get(k as usize);
        let layer = |l: Layer| match l {
            Layer::Behind => "behind",
            Layer::Text => "text",
            Layer::Front => "front",
        };
        let rank = |l: &str| match l {
            "behind" => 0,
            "text" => 1,
            _ => 2,
        };
        let mut out: Vec<(i32, ObjectTarget)> = Vec::new();
        for o in &page.objects {
            let Some(c) = control(o.para, o.control) else {
                continue;
            };
            out.push((
                o.z,
                ObjectTarget {
                    section,
                    steps: Vec::new(),
                    para: o.para,
                    control: o.control,
                    kind: control_kind(c),
                    layer: layer(o.layer),
                    page: i as u32,
                    x: o.rect.x,
                    y: o.rect.y,
                    width: o.rect.w,
                    height: o.rect.h,
                },
            ));
        }
        for t in &page.tables {
            let Some(c) = control(t.para, t.control) else {
                continue;
            };
            let width = self
                .layout
                .para(section, t.para)
                .table(t.control)
                .map_or(0, |tl| tl.width);
            let (l, z) = match c.object_common() {
                Some(oc) if !oc.treat_as_char() => (hm_layout::objects::layer_of(&oc), oc.z_order),
                _ => (Layer::Text, 0),
            };
            out.push((
                z,
                ObjectTarget {
                    section,
                    steps: Vec::new(),
                    para: t.para,
                    control: t.control,
                    kind: "table",
                    layer: layer(l),
                    page: i as u32,
                    x: t.x,
                    y: t.y,
                    width,
                    height: t.height(),
                },
            ));
        }
        out.sort_by_key(|(z, o)| (rank(o.layer), *z));
        out.into_iter().map(|(_, o)| o).collect()
    }

    pub(super) fn check(&self, pos: &Pos) -> Result<(u32, u32)> {
        let (lo, hi) = caret_range(&self.editor.doc, &pos.list, pos.para).ok_or_else(|| {
            EngineError::new("BAD_ADDRESS", "no such paragraph list or paragraph")
        })?;
        Ok((lo, hi))
    }

    /// Pages likely to show a position, nearest first: the hint, then the
    /// pages drawing its section's top paragraph.
    pub(super) fn pages_of(&self, pos: &Pos, hint: Option<u32>) -> Vec<usize> {
        let s = pos.list.section;
        let para = pos.list.steps.first().map_or(pos.para, |c| c.para);
        let mut v: Vec<(u32, usize)> = self
            .layout
            .pages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.section == s)
            .filter_map(|(i, p)| {
                let paras = p
                    .lines
                    .iter()
                    .map(|l| l.para)
                    .chain(p.tables.iter().map(|t| t.para))
                    .chain(p.objects.iter().map(|o| o.para));
                let (lo, hi) = paras.fold((u32::MAX, 0), |(lo, hi), q| (lo.min(q), hi.max(q)));
                if lo > hi {
                    return None;
                }
                let d = if para < lo {
                    lo - para
                } else {
                    para.saturating_sub(hi)
                };
                (d <= 1).then_some((d, i))
            })
            .collect();
        v.sort();
        let mut out: Vec<usize> = hint.map(|h| h as usize).into_iter().collect();
        out.extend(
            v.into_iter()
                .map(|(_, i)| i)
                .filter(|i| Some(*i as u32) != hint),
        );
        // Furniture shows on pages far from its anchor: the first page of
        // the section as a last resort.
        if !pos.list.steps.is_empty()
            && let Some(first) = self.layout.pages.iter().position(|p| p.section == s)
            && !out.contains(&first)
        {
            out.push(first);
        }
        out
    }

    pub(super) fn caret(
        &mut self,
        pos: &Pos,
        affinity: Affinity,
        hint: Option<u32>,
    ) -> Result<Option<Caret>> {
        for i in self.pages_of(pos, hint) {
            if i >= self.layout.pages.len() {
                continue;
            }
            if let Some(c) = self.index(i)?.caret(pos, affinity) {
                return Ok(Some(Caret {
                    page: i as u32,
                    x: c[0],
                    y: c[1],
                    height: c[2],
                }));
            }
        }
        Ok(None)
    }

    /// The laid-out lines of the paragraph at `pos`, in order. A
    /// paragraph split over pages lists the lines of each page it reaches.
    pub(super) fn lines(&mut self, pos: &Pos) -> Result<Vec<types::LineInfo>> {
        let mut out: Vec<types::LineInfo> = Vec::new();
        let s = pos.list.section;
        let top = pos.list.steps.first().map_or(pos.para, |c| c.para);
        let furniture = !pos.list.steps.is_empty();
        for i in 0..self.layout.pages.len() {
            let page = &self.layout.pages[i];
            if page.section != s {
                continue;
            }
            // Body text only shows on pages that hold its top paragraph.
            if !furniture {
                let holds = page.lines.iter().any(|l| l.para == top)
                    || page.tables.iter().any(|t| t.para == top)
                    || page.objects.iter().any(|o| o.para == top);
                if !holds {
                    continue;
                }
            }
            let index = self.index(i)?;
            for l in &index.lines {
                if l.list != pos.list || l.para != pos.para {
                    continue;
                }
                if out.iter().any(|o| o.line == l.line) {
                    continue;
                }
                out.push(types::LineInfo {
                    line: l.line,
                    page: i as u32,
                    y: l.y,
                    height: l.height,
                    baseline: l.baseline,
                    start: l.start(),
                    end: l.end(),
                    left: l.left(),
                    right: l.right(),
                });
            }
            // Furniture repeats on every page: the first page showing it
            // is enough.
            if furniture && !out.is_empty() {
                break;
            }
        }
        out.sort_by_key(|l| l.line);
        Ok(out)
    }

    /// The table control at `para`, `control` of a list.
    pub(super) fn table_at(
        &self,
        list: &hm_edit::ListPath,
        para: u32,
        control: u32,
    ) -> Result<&hm_model::Table> {
        match hm_edit::ops::list(&self.editor.doc, list)
            .and_then(|l| l.get(para as usize))
            .and_then(|p| p.controls.get(control as usize))
            .map(|c| &c.kind)
        {
            Some(hm_model::ControlKind::Table(t)) => Ok(t),
            _ => Err(EngineError::new("BAD_ADDRESS", "no such table")),
        }
    }

    /// Cell origins and list addresses with their visible fragments on each page.
    pub(super) fn table_info(
        &mut self,
        table: types::TableAddress,
        page: Option<u32>,
    ) -> Result<String> {
        if let Some(page) = page {
            if page as usize >= self.layout.pages.len() {
                return Err(EngineError::new("NO_SUCH_PAGE", "no such table query page"));
            }
        }
        let path = tables::path(&table);
        let t = self.table_at(&path, table.para, table.control)?;
        let (rows, columns) = (t.rows, t.cols);
        let cells: Vec<_> = t
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut steps = table.steps.clone();
                steps.push(types::StepJs {
                    para: table.para,
                    control: table.control,
                    index: i as u32,
                });
                (
                    types::CellRef {
                        row: c.row,
                        column: c.col,
                    },
                    c.row_span,
                    c.col_span,
                    steps,
                    c as *const hm_model::Cell as usize,
                )
            })
            .collect();
        let furniture = table.steps.first().is_some_and(|step| {
            self.editor
                .doc
                .sections
                .get(table.section as usize)
                .and_then(|s| s.paras.get(step.para as usize))
                .and_then(|p| p.controls.get(step.control as usize))
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        hm_model::ControlKind::Section(_)
                            | hm_model::ControlKind::HeaderFooter(_)
                            | hm_model::ControlKind::Note(_)
                    )
                })
        });
        let pages = match page {
            Some(p) => vec![p as usize],
            None if furniture => self
                .layout
                .pages
                .iter()
                .enumerate()
                .filter(|(_, p)| p.section == table.section)
                .map(|(i, _)| i)
                .collect(),
            None => self.pages_of(
                &Pos {
                    list: path,
                    para: table.para,
                    offset: 0,
                },
                None,
            ),
        };
        let mut fragments = std::collections::HashMap::<usize, Vec<Rect>>::new();
        let wanted: std::collections::HashSet<_> = cells.iter().map(|c| c.4).collect();
        for page in pages {
            let index = self.index(page)?;
            for c in &index.cells {
                if wanted.contains(&c.cell) {
                    fragments.entry(c.cell).or_default().push(Rect {
                        page: page as u32,
                        x: c.x,
                        y: c.y,
                        width: c.width,
                        height: c.height,
                    });
                }
            }
        }
        let mut out = Vec::new();
        let mut boxes = std::collections::BTreeMap::<u32, [i32; 4]>::new();
        for (at, row_span, column_span, steps, key) in cells {
            let mut rects = fragments.remove(&key).unwrap_or_default();
            rects.sort_by_key(|r| (r.page, r.y, r.x));
            for r in &rects {
                let b = boxes
                    .entry(r.page)
                    .or_insert([r.x, r.y, r.x + r.width, r.y + r.height]);
                b[0] = b[0].min(r.x);
                b[1] = b[1].min(r.y);
                b[2] = b[2].max(r.x + r.width);
                b[3] = b[3].max(r.y + r.height);
            }
            out.push(serde_json::json!({ "row": at.row, "column": at.column,
                "rowSpan": row_span, "columnSpan": column_span,
                "list": { "section": table.section, "steps": steps }, "rects": rects }));
        }
        let rects: Vec<_> = boxes
            .into_iter()
            .map(|(page, b)| Rect {
                page,
                x: b[0],
                y: b[1],
                width: b[2] - b[0],
                height: b[3] - b[1],
            })
            .collect();
        Ok(json(
            &serde_json::json!({ "table": table, "rows": rows, "columns": columns, "cells": out, "rects": rects }),
        ))
    }

    /// A selection request normalized, with its page rectangles.
    pub(super) fn selection(&mut self, req: SelectRequest) -> Result<(range::Range, Vec<Rect>)> {
        let (range, page) = match req {
            SelectRequest::Text(t) => (self.range(&t.from.pos(), &t.to.pos())?, t.page),
            SelectRequest::Cells(c) => {
                if c.kind != "cells" {
                    return Err(EngineError::new(
                        "BAD_REQUEST",
                        format!("unknown kind {:?}", c.kind),
                    ));
                }
                let list = Address {
                    section: c.table.section,
                    steps: c.table.steps.clone(),
                    para: 0,
                    offset: 0,
                }
                .pos()
                .list;
                let t = self.table_at(&list, c.table.para, c.table.control)?;
                let rows = (c.from.row.min(c.to.row), c.from.row.max(c.to.row));
                let cols = (
                    c.from.column.min(c.to.column),
                    c.from.column.max(c.to.column),
                );
                let (rows, cols) = range::grow(t, rows, cols);
                (
                    range::Range::Cells {
                        list,
                        para: c.table.para,
                        control: c.table.control,
                        rows,
                        cols,
                    },
                    c.page,
                )
            }
        };
        let mut out = Vec::new();
        let rect = |page: usize, [x, y, w, h]: [i32; 4]| Rect {
            page: page as u32,
            x,
            y,
            width: w,
            height: h,
        };
        match &range {
            range::Range::Text(from, to) => {
                let pages: Vec<usize> = match page {
                    Some(p) => vec![p as usize],
                    None => {
                        let a = self.caret(from, Affinity::Downstream, None)?;
                        let b = self.caret(to, Affinity::Upstream, None)?;
                        match (a, b) {
                            (Some(a), Some(b)) => (a.page as usize..=b.page as usize).collect(),
                            (Some(a), None) => vec![a.page as usize],
                            (None, Some(b)) => vec![b.page as usize],
                            (None, None) => Vec::new(),
                        }
                    }
                };
                for &p in &pages {
                    for r in self.index(p)?.selection(from, to) {
                        out.push(rect(p, r));
                    }
                }
                let doc = &self.editor.doc;
                let paras = hm_edit::ops::list(doc, &from.list);
                // Tables and objects of the body inside the range show whole.
                let inside = |para: u32, control: u32| -> bool {
                    let Some(p) = paras.and_then(|l| l.get(para as usize)) else {
                        return false;
                    };
                    p.units().any(|(pos, u)| {
                        matches!(u, Unit::Extended { index, .. } if index == control)
                            && (from.para, from.offset) <= (para, pos)
                            && (para, pos) < (to.para, to.offset)
                    })
                };
                for p in pages {
                    let Some(page) = self.layout.pages.get(p) else {
                        continue;
                    };
                    if !from.list.steps.is_empty() || page.section != from.list.section {
                        continue;
                    }
                    for t in &page.tables {
                        if inside(t.para, t.control) {
                            let w = self
                                .layout
                                .para(page.section, t.para)
                                .table(t.control)
                                .map_or(0, |tl| tl.width);
                            out.push(rect(p, [t.x, t.y, w, t.height()]));
                        }
                    }
                    for o in &page.objects {
                        if inside(o.para, o.control) {
                            out.push(rect(p, [o.rect.x, o.rect.y, o.rect.w, o.rect.h]));
                        }
                    }
                }
            }
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let t = self.table_at(list, *para, *control)?;
                let cells = range::block_cells(t, *rows, *cols);
                if list.steps.is_empty() {
                    // Body tables: each cell box on each fragment showing it.
                    let section = list.section;
                    let wanted: std::collections::HashSet<_> = cells
                        .iter()
                        .filter_map(|&k| t.cells.get(k))
                        .map(|c| c as *const hm_model::Cell as usize)
                        .collect();
                    for (i, pg) in self.layout.pages.iter().enumerate() {
                        if pg.section != section
                            || page.is_some_and(|q| q as usize != i)
                            || !pg
                                .tables
                                .iter()
                                .any(|f| f.para == *para && f.control == *control)
                        {
                            continue;
                        }
                        for c in hm_render::page_cells(
                            &self.editor.doc,
                            &self.layout,
                            &self.fonts.borrow(),
                            i,
                        ) {
                            if wanted.contains(&c.cell) {
                                out.push(rect(i, [c.x, c.y, c.width, c.height]));
                            }
                        }
                    }
                }
                // A table set as a character sits on its line, its bottom on
                // the baseline when the line has room above it.
                if out.is_empty()
                    && list.steps.is_empty()
                    && let Some(tl) = self.layout.para(list.section, *para).table(*control)
                {
                    let height = tl.height;
                    let boxes: Vec<[i32; 4]> = cells
                        .iter()
                        .filter_map(|&k| tl.cells.get(k).map(|c| [c.x, c.y, c.w, c.h]))
                        .collect();
                    let at = hm_edit::ops::list(&self.editor.doc, list)
                        .and_then(|l| l.get(*para as usize))
                        .and_then(|p| range::control_span(p, *control))
                        .map_or(0, |s| s.0);
                    let pages: Vec<usize> = match page {
                        Some(p) => vec![p as usize],
                        None => self.pages_of(
                            &Pos {
                                list: list.clone(),
                                para: *para,
                                offset: at,
                            },
                            None,
                        ),
                    };
                    for p in pages {
                        let index = self.index(p)?;
                        let Some(l) = index.lines.iter().find(|l| {
                            l.list == *list && l.para == *para && l.start() <= at && at < l.end()
                        }) else {
                            continue;
                        };
                        // Never above its line's top.
                        let (x, y) = (l.x_of(at), l.y + (l.baseline - height).max(0));
                        for [cx, cy, w, h] in &boxes {
                            out.push(rect(p, [x + cx, y + cy, *w, *h]));
                        }
                    }
                }
                if out.is_empty() {
                    // Nested tables: the lines of each cell, boxed per page.
                    let pages: Vec<usize> = match page {
                        Some(p) => vec![p as usize],
                        None => self.pages_of(
                            &Pos {
                                list: list.clone(),
                                para: *para,
                                offset: 0,
                            },
                            None,
                        ),
                    };
                    for p in pages {
                        let index = self.index(p)?;
                        for &k in &cells {
                            let mut steps = list.steps.clone();
                            steps.push(hm_edit::Step {
                                para: *para,
                                control: *control,
                                index: k as u32,
                            });
                            let cell = hm_edit::ListPath {
                                section: list.section,
                                steps,
                            };
                            let mut b = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                            for l in index.lines.iter().filter(|l| l.list == cell) {
                                b = (
                                    b.0.min(l.left()),
                                    b.1.min(l.y),
                                    b.2.max(l.right()),
                                    b.3.max(l.y + l.height),
                                );
                            }
                            if b.0 <= b.2 {
                                out.push(rect(p, [b.0, b.1, b.2 - b.0, b.3 - b.1]));
                            }
                        }
                    }
                }
            }
        }
        Ok((range, out))
    }

    /// A selection from `a` to `b` normalized ([`range::normalize`]).
    pub(super) fn range(&self, a: &Pos, b: &Pos) -> Result<range::Range> {
        self.check(a)?;
        self.check(b)?;
        range::normalize(&self.editor.doc, a, b).map_err(|e| EngineError::new("BAD_ADDRESS", e))
    }

    /// The single-list spans a selection covers: the range itself, or the
    /// whole content of each cell of a block.
    pub(super) fn spans(&self, a: &Pos, b: &Pos) -> Result<Vec<(Pos, Pos)>> {
        Self::spans_in(&self.editor.doc, a, b)
    }

    pub(super) fn spans_in(doc: &hm_model::Document, a: &Pos, b: &Pos) -> Result<Vec<(Pos, Pos)>> {
        if a.list.section != b.list.section && a.list.steps.is_empty() && b.list.steps.is_empty() {
            if a.list.section > b.list.section {
                return Err(EngineError::new("BAD_ADDRESS", "reversed section range"));
            }
            let mut spans = Vec::new();
            for section in a.list.section..=b.list.section {
                let list = hm_edit::ListPath {
                    section,
                    steps: Vec::new(),
                };
                let paras = hm_edit::ops::list(doc, &list)
                    .ok_or_else(|| EngineError::new("BAD_ADDRESS", "no such section"))?;
                let last = paras
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| EngineError::new("BAD_ADDRESS", "empty section"))?
                    as u32;
                let from = if section == a.list.section {
                    a.clone()
                } else {
                    Pos {
                        list: list.clone(),
                        para: 0,
                        offset: hm_edit::ops::lead(&paras[0]),
                    }
                };
                let to = if section == b.list.section {
                    b.clone()
                } else {
                    Pos {
                        list,
                        para: last,
                        offset: paras[last as usize].len().saturating_sub(1),
                    }
                };
                spans.extend(Self::spans_in(doc, &from, &to)?);
            }
            return Ok(spans);
        }
        for pos in [a, b] {
            if caret_range(doc, &pos.list, pos.para).is_none() {
                return Err(EngineError::new(
                    "BAD_ADDRESS",
                    "no such paragraph list or paragraph",
                ));
            }
        }
        let range = range::normalize(doc, a, b).map_err(|e| EngineError::new("BAD_ADDRESS", e))?;
        Ok(match range {
            range::Range::Text(from, to) => vec![(from, to)],
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let Some(hm_model::ControlKind::Table(t)) = hm_edit::ops::list(doc, &list)
                    .and_then(|l| l.get(para as usize))
                    .and_then(|p| p.controls.get(control as usize))
                    .map(|c| &c.kind)
                else {
                    return Err(EngineError::new("BAD_ADDRESS", "no such table"));
                };
                range::block_cells(t, rows, cols)
                    .into_iter()
                    .map(|i| {
                        let mut steps = list.steps.clone();
                        steps.push(hm_edit::Step {
                            para,
                            control,
                            index: i as u32,
                        });
                        let cell = hm_edit::ListPath {
                            section: list.section,
                            steps,
                        };
                        let paras = &t.cells[i].paras;
                        let last = paras.len().saturating_sub(1);
                        let from = Pos {
                            list: cell.clone(),
                            para: 0,
                            offset: paras.first().map_or(0, hm_edit::ops::lead),
                        };
                        let to = Pos {
                            list: cell,
                            para: last as u32,
                            offset: paras.last().map_or(0, |p| p.len().saturating_sub(1)),
                        };
                        (from, to)
                    })
                    .collect()
            }
        })
    }

    /// The caret position after `motion`, with its affinity.
    pub(super) fn navigate(&mut self, req: &NavigateRequest) -> Result<Option<(Pos, Affinity)>> {
        let pos = req.position.pos();
        let (lo, hi) = self.check(&pos)?;
        let stops = |doc: &hm_model::Document, pos: &Pos| -> Vec<u32> {
            hm_edit::ops::list(doc, &pos.list)
                .and_then(|l| l.get(pos.para as usize))
                .map_or_else(Vec::new, text::stops)
        };
        let doc = &self.editor.doc;
        let count = hm_edit::ops::list(doc, &pos.list).map_or(0, |l| l.len() as u32);
        match req.motion {
            Motion::Left | Motion::Right => {
                let s = stops(doc, &pos);
                let left = req.motion == Motion::Left;
                let next = if left {
                    s.iter().rev().find(|&&q| q < pos.offset).copied()
                } else {
                    s.iter().find(|&&q| q > pos.offset).copied()
                };
                if let Some(offset) = next {
                    return Ok(Some((Pos { offset, ..pos }, Affinity::Downstream)));
                }
                let para = if left {
                    pos.para.checked_sub(1)
                } else {
                    (pos.para + 1 < count).then_some(pos.para + 1)
                };
                let Some(para) = para else {
                    return Ok(Some((pos, req.affinity)));
                };
                let target = Pos {
                    para,
                    ..pos.clone()
                };
                let (lo, hi) = caret_range(doc, &target.list, para).unwrap_or((0, 0));
                let offset = if left { hi } else { lo };
                Ok(Some((Pos { offset, ..target }, Affinity::Downstream)))
            }
            Motion::LineStart | Motion::LineEnd => {
                let Some(c) = self.caret(&pos, req.affinity, req.page)? else {
                    return Ok(None);
                };
                let built = self.index(c.page as usize)?;
                let Some(line) = built.line_of(&pos, req.affinity) else {
                    return Ok(None);
                };
                if req.motion == Motion::LineStart {
                    let offset = line.start().max(lo);
                    Ok(Some((Pos { offset, ..pos }, Affinity::Downstream)))
                } else if line.end() > hi {
                    Ok(Some((Pos { offset: hi, ..pos }, Affinity::Downstream)))
                } else {
                    let offset = line.end();
                    Ok(Some((Pos { offset, ..pos }, Affinity::Upstream)))
                }
            }
            Motion::Up | Motion::Down => {
                let Some(c) = self.caret(&pos, req.affinity, req.page)? else {
                    return Ok(None);
                };
                let mut x = req.preferred_x.map_or(c.x, |v| v.round() as i32);
                let up = req.motion == Motion::Up;
                let built = self.index(c.page as usize)?;
                let here = built
                    .line_of(&pos, req.affinity)
                    .map(|l| (l.y, l.height, l.list.clone(), l.origin));
                let (top, bottom) = here
                    .as_ref()
                    .map_or((c.y, c.y + c.height), |(y, h, _, _)| (*y, y + h));
                // The nearest line above or below that overlaps in x, else
                // the nearest one at all.
                let beyond = |l: &Line| {
                    if up {
                        l.y + l.height <= top
                    } else {
                        l.y >= bottom
                    }
                };
                let gap = |l: &Line| {
                    let dy = if up {
                        top - (l.y + l.height)
                    } else {
                        l.y - bottom
                    };
                    let dx = if x < l.left() {
                        l.left() - x
                    } else {
                        (x - l.right()).max(0)
                    };
                    (dx > 0, dy, dx)
                };
                // Headers, footers and notes are entered on purpose, not by moving
                // past the body's edge; inside one the caret stays in its list.
                let doc = &self.editor.doc;
                let in_furniture = here
                    .as_ref()
                    .is_some_and(|(_, _, list, _)| crate::furniture_list(doc, list));
                let own = here.as_ref().map(|(_, _, list, _)| list.clone());
                let allowed = |doc: &hm_model::Document, l: &Line| match &own {
                    Some(list) if in_furniture => l.list == *list,
                    _ => !crate::furniture_list(doc, &l.list),
                };
                // The next line in the caret's own column of its own list,
                // however short or empty; past a cell's last line the cell
                // above or below in the caret's column; only past the end of
                // that flow (a column's foot, a table's edge) any line that
                // lies under x.
                let cell_paths = here
                    .as_ref()
                    .filter(|(_, _, list, _)| !list.steps.is_empty())
                    .map(|_| objects::cell_lists(doc));
                let pick = |idx: &PageIndex| -> Option<(usize, Option<i32>)> {
                    let flow = here.as_ref().and_then(|(_, _, list, origin)| {
                        idx.lines
                            .iter()
                            .enumerate()
                            .filter(|(_, l)| beyond(l) && l.list == *list && l.origin == *origin)
                            .min_by_key(|(_, l)| {
                                let (_, dy, dx) = gap(l);
                                (dy, dx)
                            })
                            .map(|(i, _)| i)
                    });
                    if let Some(i) = flow {
                        return Some((i, None));
                    }
                    if let (Some((_, _, list, _)), Some(paths)) = (&here, &cell_paths)
                        && let Some(found) = Self::across_cells(idx, paths, list, x, up)
                    {
                        return Some(found);
                    }
                    idx.lines
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| beyond(l) && allowed(doc, l))
                        .min_by_key(|(_, l)| gap(l))
                        .map(|(i, _)| (i, None))
                };
                let mut target = pick(&built).map(|(i, cx)| {
                    if let Some(cx) = cx {
                        x = cx;
                    }
                    (c.page as usize, i)
                });
                if target.is_none() {
                    let next = if up {
                        (c.page as usize).checked_sub(1)
                    } else {
                        Some(c.page as usize + 1).filter(|&p| p < self.layout.pages.len())
                    };
                    if let Some(p) = next {
                        let idx = self.index(p)?;
                        let doc = &self.editor.doc;
                        let edge = if up {
                            idx.lines
                                .iter()
                                .enumerate()
                                .filter(|(_, l)| allowed(doc, l))
                                .max_by_key(|(_, l)| l.y + l.height)
                        } else {
                            idx.lines
                                .iter()
                                .enumerate()
                                .filter(|(_, l)| allowed(doc, l))
                                .min_by_key(|(_, l)| l.y)
                        };
                        target = edge.map(|(i, _)| (p, i));
                    }
                }
                let Some((p, mut t)) = target else {
                    return Ok(Some((pos, req.affinity)));
                };
                let idx = self.index(p)?;
                // A line holding a table passes the caret into the table: into
                // the cell under x (the nearest one when x is beside a table
                // that fills its line), from the side the caret comes from, and
                // on into tables nested there.
                let mut paths = None;
                loop {
                    let host = &idx.lines[t];
                    let paths = paths.get_or_insert_with(|| objects::cell_lists(&self.editor.doc));
                    let depth = host.list.steps.len();
                    let cells: Vec<_> = idx
                        .cells
                        .iter()
                        .filter_map(|b| paths.get(&b.cell).map(|l| (b, l)))
                        .filter(|(_, l)| {
                            l.section == host.list.section
                                && l.steps.len() == depth + 1
                                && l.steps[..depth] == host.list.steps[..]
                                && l.steps[depth].para == host.para
                        })
                        .collect();
                    let Some(left) = cells.iter().map(|(b, _)| b.x).min() else {
                        break;
                    };
                    // Only a table that starts on this line, inline in it or
                    // set below it; not one anchored here but placed apart.
                    let top = cells.iter().map(|(b, _)| b.y).min().unwrap_or(host.y);
                    if top < host.y || top > host.y + host.height {
                        break;
                    }
                    let right = cells
                        .iter()
                        .map(|(b, _)| b.x + b.width)
                        .max()
                        .unwrap_or(left);
                    // Every item with width on the line overlaps the table.
                    let only_table = host
                        .stops
                        .windows(2)
                        .filter(|w| w[1].1 > w[0].1)
                        .all(|w| w[1].1 > left && w[0].1 < right);
                    if !only_table && (x < left || x >= right) {
                        break;
                    }
                    let dx = |b: &hm_render::CellBox| {
                        if x < b.x {
                            b.x - x
                        } else {
                            (x - (b.x + b.width - 1)).max(0)
                        }
                    };
                    let cell = cells
                        .iter()
                        .min_by_key(|(b, _)| (if up { -(b.y + b.height) } else { b.y }, dx(b)))
                        .map(|(_, l)| (*l).clone());
                    let Some(cell) = cell else { break };
                    let first = idx
                        .lines
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| l.list == cell)
                        .min_by_key(|(_, l)| if up { -(l.y + l.height) } else { l.y });
                    match first {
                        Some((i, _)) => t = i,
                        None => break,
                    }
                }
                let line = &idx.lines[t];
                let (list, para, y, h) = (line.list.clone(), line.para, line.y, line.height);
                let hit = idx.hit(&self.editor.doc, x, y + h / 2, &|l: &Line| {
                    l.list == list && l.para == para && l.y == y && l.height == h
                });
                Ok(hit.map(|(pos, aff, _, _)| (pos, aff)))
            }
        }
    }
}

#[wasm_bindgen]
impl Doc {
    /// A point to a position (`Located`), or a position to its caret
    /// (`Caret`); `null` when nothing editable is there.
    pub fn locate(&mut self, request: &str) -> Result<String> {
        let req: LocateRequest = parse(request)?;
        let d = self.inner()?;
        match req {
            LocateRequest::Point {
                page,
                x,
                y,
                body,
                within,
            } => {
                let (x, y) = (x.round() as i32, y.round() as i32);
                let built = d.index(page as usize)?;
                let within = within.map(|l| {
                    Address {
                        section: l.section,
                        steps: l.steps,
                        para: 0,
                        offset: 0,
                    }
                    .pos()
                    .list
                });
                let doc = &d.editor.doc;
                let keep = |l: &Line| {
                    if let Some(w) = &within {
                        return l.list.section == w.section && l.list.steps.starts_with(&w.steps);
                    }
                    !body || !furniture_list(doc, &l.list)
                };
                // A point in a table cell finds that cell's text, even where
                // the line holding the table lies nearer than the cell's
                // own (short or empty) lines.
                let cell = if within.is_none() {
                    let cells: Vec<_> = built
                        .cells
                        .iter()
                        .filter(|c| x >= c.x && x < c.x + c.width && y >= c.y && y < c.y + c.height)
                        .collect();
                    if cells.is_empty() {
                        None
                    } else {
                        let lists = objects::cell_lists(doc);
                        cells
                            .iter()
                            .filter_map(|c| lists.get(&c.cell))
                            .max_by_key(|l| l.steps.len())
                            .cloned()
                    }
                } else {
                    None
                };
                let in_cell = |l: &Line| {
                    cell.as_ref().is_some_and(|c| {
                        l.list.section == c.section && l.list.steps.starts_with(&c.steps)
                    }) && keep(l)
                };
                let found = cell
                    .as_ref()
                    .and_then(|_| built.hit(doc, x, y, &in_cell))
                    .or_else(|| built.hit(doc, x, y, &keep));
                let Some((pos, affinity, c, inside)) = found else {
                    return Ok("null".into());
                };
                // The topmost object under the point, unless it lies behind
                // text the point is on.
                let object = d
                    .objects(page as usize)
                    .into_iter()
                    .rev()
                    .find(|o| x >= o.x && x < o.x + o.width && y >= o.y && y < o.y + o.height)
                    .filter(|o| o.layer != "behind" || !inside);
                Ok(json(&Located {
                    position: Address::of(&pos),
                    affinity,
                    caret: Caret {
                        page,
                        x: c[0],
                        y: c[1],
                        height: c[2],
                    },
                    object,
                }))
            }
            LocateRequest::Caret {
                position,
                affinity,
                page,
            } => {
                let pos = position.pos();
                d.check(&pos)?;
                Ok(match d.caret(&pos, affinity, page)? {
                    Some(c) => json(&c),
                    None => "null".into(),
                })
            }
        }
    }

    /// The body's floating objects and tables on page `i`
    /// (`ObjectTarget[]`), bottom to top.
    pub fn objects(&mut self, i: usize) -> Result<String> {
        let d = self.inner()?;
        if i >= d.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        Ok(json(&d.objects(i)))
    }

    /// Every object and table page `i` draws, in any paragraph list
    /// (`PageObject[]`), bottom to top.
    #[wasm_bindgen(js_name = pageObjects)]
    pub fn page_objects(&mut self, i: usize) -> Result<String> {
        let d = self.inner()?;
        if i >= d.layout.pages.len() {
            return Err(EngineError::new("NO_SUCH_PAGE", format!("no page {i}")));
        }
        let fonts = d.fonts.borrow();
        Ok(json(&objects::page_objects(
            &d.editor.doc,
            &d.layout,
            &fonts,
            i,
        )))
    }

    /// Selection rectangles (`Rect[]`) for `[from, to)` in one list.
    pub fn select(&mut self, request: &str) -> Result<String> {
        let req: SelectRequest = parse(request)?;
        let (_, rects) = self.inner()?.selection(req)?;
        Ok(json(&rects))
    }

    /// Like [`Doc::select`], with the selection as normalized
    /// (`Selected`): the text range in the common list, or the block of
    /// cells with each cell's list.
    #[wasm_bindgen(js_name = selectRange)]
    pub fn select_range(&mut self, request: &str) -> Result<String> {
        let req: SelectRequest = parse(request)?;
        let d = self.inner()?;
        let (range, rects) = d.selection(req)?;
        let range = match range {
            range::Range::Text(from, to) => types::SelectedRange::Text {
                from: Address::of(&from),
                to: Address::of(&to),
            },
            range::Range::Cells {
                list,
                para,
                control,
                rows,
                cols,
            } => {
                let t = d.table_at(&list, para, control)?;
                let step = |p: &hm_edit::Step| types::StepJs {
                    para: p.para,
                    control: p.control,
                    index: p.index,
                };
                let cells = range::block_cells(t, rows, cols)
                    .into_iter()
                    .map(|i| {
                        let c = &t.cells[i];
                        let mut steps: Vec<types::StepJs> = list.steps.iter().map(step).collect();
                        steps.push(types::StepJs {
                            para,
                            control,
                            index: i as u32,
                        });
                        types::SelectedCell {
                            row: c.row,
                            column: c.col,
                            row_span: c.row_span.max(1),
                            column_span: c.col_span.max(1),
                            list: types::ListAddressOut {
                                section: list.section,
                                steps,
                            },
                        }
                    })
                    .collect();
                types::SelectedRange::Cells {
                    table: types::TableAddress {
                        section: list.section,
                        steps: list.steps.iter().map(step).collect(),
                        para,
                        control,
                    },
                    from: types::CellRef {
                        row: rows.0,
                        column: cols.0,
                    },
                    to: types::CellRef {
                        row: rows.1,
                        column: cols.1,
                    },
                    cells,
                }
            }
        };
        Ok(json(&types::Selected { range, rects }))
    }

    /// The caret after a movement (`Located`), `null` when the position is
    /// on no page.
    pub fn navigate(&mut self, request: &str) -> Result<String> {
        let req: NavigateRequest = parse(request)?;
        let d = self.inner()?;
        let Some((pos, affinity)) = d.navigate(&req)? else {
            return Ok("null".into());
        };
        let Some(caret) = d.caret(&pos, affinity, req.page)? else {
            return Ok("null".into());
        };
        Ok(json(&Located {
            position: Address::of(&pos),
            affinity,
            caret,
            object: None,
        }))
    }
}
