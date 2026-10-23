//! Incremental layout equals layout from scratch after random edits
//! (`HEEMANG_CORPUS` and `HEEMANG_FONTS`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_edit::{Editor, ListPath, Op, Pos, Step};
use hm_fonts::FontSystem;
use hm_model::{Document, Paragraph, SharedBytes, Unit};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn setup(rel: &str) -> Option<(Document, FontSystem)> {
    let (Some(root), Some(fonts)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return None;
    };
    let mut fs = FontSystem::new();
    for dir in fonts.split(':') {
        let mut files = Vec::new();
        walk(Path::new(dir), &mut files);
        for f in files {
            let ext = f
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                fs.add(
                    f.display().to_string(),
                    Arc::new(std::fs::read(&f).unwrap()),
                );
            }
        }
    }
    let bytes = SharedBytes::from(std::fs::read(PathBuf::from(root).join(rel)).ok()?);
    let doc = if hm_hwpx::is_hwpx(&bytes) {
        hm_hwpx::read(bytes).ok()?
    } else {
        hm_hwp5::read(bytes).ok()?
    };
    Some((doc, fs))
}

/// The layout with style table indices replaced by the styles they name
/// (incremental layouts keep a longer table, so indices differ).
fn fingerprint(l: &hm_layout::DocLayout) -> String {
    let style = |i: u32| format!("{:?}", l.styles.get(i as usize));
    let para = |p: &hm_layout::ParaLayout| {
        let mut s = String::new();
        for line in &p.lines {
            s.push_str(&format!(
                "{} {} {} {} {} {} {} {};",
                line.start,
                line.end,
                line.x,
                line.seg_width,
                line.height,
                line.baseline,
                line.spacing,
                line.used
            ));
        }
        s.push_str(&format!("{:?}", p.measured.advances));
        s
    };
    let mut s = String::new();
    for page in &l.pages {
        s.push_str(&format!(
            "{} {} {} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?}|",
            page.section,
            page.width,
            page.height,
            page.body,
            page.columns,
            page.lines,
            page.objects,
            page.tables,
            page.number,
            page.rules,
            page.border
        ));
        for m in &page.marks {
            for &(c, st, x) in &m.glyphs {
                s.push_str(&format!("{c}{}{x};", style(st)));
            }
            s.push_str(&format!("@{}|", m.baseline));
        }
        for f in &page.frames {
            s.push_str(&format!(
                "{:?} {:?} {:?} {:?} {:?}",
                f.kind, f.rect, f.lines, f.objects, f.tables
            ));
            for p in &f.paras {
                s.push_str(&para(p));
            }
        }
    }
    for sec in &l.sections {
        for p in &sec.paras {
            s.push_str(&para(p));
        }
    }
    s
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

/// Unit starts where text may be edited.
fn offsets(p: &Paragraph) -> Vec<u32> {
    let lead = hm_edit::ops::lead(p);
    p.units()
        .filter(|(pos, u)| *pos >= lead && matches!(u, Unit::Char(_) | Unit::CharControl(13)))
        .map(|(pos, _)| pos)
        .collect()
}

/// Lists to edit: the body of section 0 and the cells of its first tables.
fn lists(doc: &Document) -> Vec<ListPath> {
    let mut out = vec![ListPath::default()];
    for (pi, p) in doc.sections[0].paras.iter().enumerate() {
        for (ci, c) in p.controls.iter().enumerate() {
            if let Some(t) = c.as_table() {
                for k in 0..t.cells.len().min(3) {
                    out.push(ListPath {
                        section: 0,
                        steps: vec![Step {
                            para: pi as u32,
                            control: ci as u32,
                            index: k as u32,
                        }],
                    });
                }
            }
        }
        if out.len() > 12 {
            break;
        }
    }
    out
}

fn random_op(doc: &Document, rng: &mut Rng) -> Option<Op> {
    let ls = lists(doc);
    let list = ls[rng.next(ls.len() as u64) as usize].clone();
    let paras = hm_edit::ops::list(doc, &list)?;
    let pi = rng.next(paras.len() as u64) as u32;
    let offs = offsets(&paras[pi as usize]);
    let at = |o: u32| Pos {
        list: list.clone(),
        para: pi,
        offset: o,
    };
    let o = *offs.get(rng.next(offs.len() as u64) as usize)?;
    Some(match rng.next(5) {
        0 | 1 => Op::InsertText {
            at: at(o),
            text: ["가", "abc ", "새로운 문장을 넣습니다. ", "1"][rng.next(4) as usize].into(),
        },
        2 => {
            let e = *offs.iter().find(|&&x| x > o && x <= o + 6).unwrap_or(&o);
            Op::Delete {
                from: at(o),
                to: at(e),
            }
        }
        3 => Op::Split {
            at: at(o),
            page_break: false,
        },
        _ => {
            // Join with the next paragraph.
            if pi + 1 >= paras.len() as u32 {
                return None;
            }
            let last = paras[pi as usize].len() - 1;
            let next = Pos {
                list: list.clone(),
                para: pi + 1,
                offset: hm_edit::ops::lead(&paras[pi as usize + 1]),
            };
            Op::Delete {
                from: at(last),
                to: next,
            }
        }
    })
}

fn check(rel: &str, seed: u64, steps: usize) {
    let Some((doc, fs)) = setup(rel) else { return };
    let mut ed = Editor::new(doc);
    let mut rng = Rng(seed);
    ed.layout(&fs);
    let mut done = 0;
    while done < steps {
        let Some(op) = random_op(&ed.doc, &mut rng) else {
            continue;
        };
        if ed.apply(op.clone()).is_err() {
            continue;
        }
        done += 1;
        let inc = ed.layout(&fs);
        let scratch = hm_layout::layout(&ed.doc, &fs);
        let (a, b) = (fingerprint(&inc), fingerprint(&scratch));
        if a != b {
            let k = a
                .bytes()
                .zip(b.bytes())
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            let lo = k.saturating_sub(300);
            panic!(
                "{rel}: step {done} after {op:?}\n inc: {}\n new: {}",
                &a[lo..(k + 200).min(a.len())],
                &b[lo..(k + 200).min(b.len())]
            );
        }
    }
    // Undo everything: the layout returns to the original.
    while ed.undo().unwrap() {}
    let inc = ed.layout(&fs);
    let scratch = hm_layout::layout(&ed.doc, &fs);
    assert_eq!(
        fingerprint(&inc),
        fingerprint(&scratch),
        "{rel}: after undo"
    );
}

#[test]
fn incremental_layout_matches_scratch_hwp() {
    check("2022년 국립국어원 업무계획.hwp", 0x9E37_79B9, 60);
}

#[test]
fn incremental_layout_matches_scratch_hwpx() {
    check(
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
        0x2545_F491,
        25,
    );
}
