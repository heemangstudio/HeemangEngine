//! A document's text as plain text or Markdown: body paragraphs in order,
//! each followed by the text of the tables, text boxes and notes it holds.
//! Headers, footers and master pages are left out.

use hm_model::{ControlKind, Document, Paragraph, Table, Unit};

/// A paragraph's characters, line breaks and tabs.
pub fn para_text(p: &Paragraph) -> String {
    let mut s = String::new();
    for (_, u) in p.units() {
        match u {
            Unit::Char(c) => s.push(c),
            Unit::CharControl(10) => s.push('\n'),
            Unit::CharControl(30) => s.push(' '),
            Unit::Inline(9) => s.push('\t'),
            _ => {}
        }
    }
    s
}

/// Plain text: paragraphs on their own lines, table rows with their cells
/// separated by tabs.
pub fn plain_text(doc: &Document) -> String {
    let mut out = String::new();
    for s in &doc.sections {
        plain(&s.paras, &mut out);
    }
    out
}

fn plain(paras: &[Paragraph], out: &mut String) {
    for p in paras {
        out.push_str(&para_text(p));
        out.push('\n');
        for c in &p.controls {
            match &c.kind {
                ControlKind::Table(t) => {
                    for row in rows(t) {
                        let cells: Vec<String> = row
                            .iter()
                            .map(|paras| {
                                let mut s = String::new();
                                plain(paras, &mut s);
                                s.trim_end().replace('\n', " ")
                            })
                            .collect();
                        out.push_str(&cells.join("\t"));
                        out.push('\n');
                    }
                }
                ControlKind::HeaderFooter(_) | ControlKind::Section(_) => {}
                _ => {
                    for list in crate::ops::control_lists(c) {
                        plain(list, out);
                    }
                }
            }
        }
    }
}

/// The cells of each row in column order, as their paragraph lists.
fn rows(t: &Table) -> Vec<Vec<&Vec<Paragraph>>> {
    let mut rows: Vec<Vec<(u16, &Vec<Paragraph>)>> = vec![Vec::new(); t.rows as usize];
    for c in &t.cells {
        if let Some(r) = rows.get_mut(c.row as usize) {
            r.push((c.col, &c.paras));
        }
    }
    rows.into_iter()
        .map(|mut r| {
            r.sort_by_key(|c| c.0);
            r.into_iter().map(|c| c.1).collect()
        })
        .collect()
}

/// Markdown: outline paragraphs as headings by level, other paragraphs as
/// paragraphs, tables as pipe tables (the first row as the header).
pub fn markdown(doc: &Document) -> String {
    let mut out = String::new();
    for s in &doc.sections {
        md(doc, &s.paras, &mut out);
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Text with Markdown's inline syntax escaped.
fn escape_inline(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '|') {
            s.push('\\');
        }
        s.push(c);
    }
    s
}

/// A paragraph's text escaped, block markers included.
fn escape(text: &str) -> String {
    let mut s = escape_inline(text).replace("\\|", "|");
    // A line starting like a block marker stays text.
    if s.starts_with(['#', '-', '+', '='])
        || s.split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    {
        s.insert(0, '\\');
    }
    s
}

fn md(doc: &Document, paras: &[Paragraph], out: &mut String) {
    for p in paras {
        let text = para_text(p);
        let line = text.trim();
        if !line.is_empty() {
            let level = doc
                .info
                .para_shapes
                .get(p.header.para_shape as usize)
                .filter(|s| s.head_kind() == 1)
                .map(|s| s.level() as usize + 1);
            let body = escape(line).replace('\n', "  \n").replace('\t', " ");
            match level {
                Some(l) => out.push_str(&format!("{} {body}\n\n", "#".repeat(l.min(6)))),
                None => out.push_str(&format!("{body}\n\n")),
            }
        }
        for c in &p.controls {
            match &c.kind {
                ControlKind::Table(t) => table(t, out),
                ControlKind::HeaderFooter(_) | ControlKind::Section(_) => {}
                _ => {
                    for list in crate::ops::control_lists(c) {
                        md(doc, list, out);
                    }
                }
            }
        }
    }
}

fn table(t: &Table, out: &mut String) {
    let rows = rows(t);
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    for (k, row) in rows.iter().enumerate() {
        out.push('|');
        for c in 0..cols {
            // Cells hold their text line by line, nested tables flattened.
            let cell = row.get(c).map_or(String::new(), |paras| {
                let mut s = String::new();
                plain(paras, &mut s);
                let lines: Vec<String> = s
                    .lines()
                    .map(|l| {
                        l.split('\t')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .filter(|l| !l.is_empty())
                    .map(|l| escape_inline(&l))
                    .collect();
                lines.join("<br>")
            });
            out.push_str(&format!(" {cell} |"));
        }
        out.push('\n');
        if k == 0 {
            out.push('|');
            out.push_str(&" --- |".repeat(cols));
            out.push('\n');
        }
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_block_markers_and_inline_syntax() {
        assert_eq!(escape_inline("a|b"), "a\\|b");
        assert_eq!(escape("a|b"), "a|b");
        assert_eq!(escape("# not a heading"), "\\# not a heading");
        assert_eq!(escape("1. item"), "\\1. item");
        assert_eq!(escape("a*b_c"), "a\\*b\\_c");
        assert_eq!(escape("가나다"), "가나다");
    }
}
