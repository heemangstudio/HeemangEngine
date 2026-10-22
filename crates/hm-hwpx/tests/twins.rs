//! HWP 5 and HWPX twins (the same document saved by Hancom in both formats)
//! read into equal models, apart from these representation differences:
//!
//! 1. Source spans and tails hold each format's own bytes or markup.
//! 2. Face lists: Hancom names a face in its local name in one format and
//!    in English in the other, and orders the lists differently, so face
//!    references are not compared.
//! 3. Border fill and BinData tables are numbered by each file's own
//!    storage order (Hancom drops unused border fills in one format), so
//!    references compare by the content they point to and the tables by
//!    their referenced entries.
//! 4. Bits HWPX has no attribute for: paragraph shape attribute 2 bit 3,
//!    object attribute bits 28 and up, shape component flip bits other
//!    than 0, 1 and 19, the shape style's reserved byte, table attribute
//!    bits 26 and up, cell list attribute bits 24 and up, page border
//!    attribute bits 5 and up.
//! 5. Inline control scratch units: tab units 4 to 6 and field end units
//!    2 to 6.
//! 6. HWPX keeps six decimals of transform matrices.
//! 7. HWP 5 records without an OWPML element: object, shape and table raw
//!    records, raw section definition children, range tags other than
//!    highlighting, the extra character shape run HWP 5 starts at a
//!    paragraph end, a missing merge flag.
//! 8. Controls the model keeps raw compare by id.
//! 9. Document properties (caret, counts) and the face details HWPX does
//!    not carry (default name, serif style).
//! 10. The width of a diagonal that draws no line.
//! 11. Stored line segments of text inside drawing objects: Hancom lays
//!     text boxes out again when it saves.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use hm_model::*;

/// Twins whose models must match exactly after normalization. Other twin
/// pairs in the corpus differ in ways Hancom introduced when converting:
/// re-measured equation sizes, text boxes laid out again, note line
/// segments without vertical positions, picture borders with no line kind,
/// and border fills it renumbered.
const PAIRS: &[(&str, &str)] = &[
    (
        "hwpx/hancom-hwp/2024년 1분기 해외직접투자 보도자료 ff.hwp",
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
    ),
    (
        "2025 행정업무운영 편람(최종).hwp",
        "2025 행정업무운영 편람(최종).hwpx",
    ),
    ("hwp3-sample13-hwp5.hwp", "hwp3-sample13-hwp5.hwpx"),
    ("hwp3-sample14-hwp5.hwp", "hwp3-sample14-hwp5.hwpx"),
    ("hwp3-sample16-hwp5.hwp", "hwp3-sample16-hwp5.hwpx"),
    (
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwp",
        "hwpx/2024년 1분기 해외직접투자 보도자료 ff.hwpx",
    ),
    (
        "hwpx/hancom-hwp/2024년 2분기 해외직접투자 보도자료ff.hwp",
        "hwpx/2024년 2분기 해외직접투자 보도자료ff.hwpx",
    ),
    (
        "hwpx/hancom-hwp/2024년 연간 해외직접투자 보도자료 _ ff.hwp",
        "hwpx/2024년 연간 해외직접투자 보도자료 _ ff.hwpx",
    ),
    (
        "hwpx/hancom-hwp/2025년 1분기 해외직접투자 보도자료f.hwp",
        "hwpx/2025년 1분기 해외직접투자 보도자료f.hwpx",
    ),
    (
        "hwpx/hancom-hwp/2025년 2분기 해외직접투자 (최종).hwp",
        "hwpx/2025년 2분기 해외직접투자 (최종).hwpx",
    ),
    ("2026_oss_rst.hwp", "hwpx/2026_oss_rst.hwpx"),
    ("hwpx/hancom-hwp/blank_hwpx.hwp", "hwpx/blank_hwpx.hwpx"),
    ("eq-002.hwp", "hwpx/eq-002.hwpx"),
    ("exam_kor.hwp", "hwpx/exam_kor.hwpx"),
    ("hwpx/hancom-hwp/form-002.hwp", "hwpx/form-002.hwpx"),
    ("hcar-001.hwp", "hwpx/hcar-001.hwpx"),
    ("hwpx/hancom-hwp/hwpx-h-01.hwp", "hwpx/hwpx-h-01.hwpx"),
    ("hwpx/hancom-hwp/hwpx-h-02.hwp", "hwpx/hwpx-h-02.hwpx"),
    ("hwpx/hancom-hwp/hwpx-h-03.hwp", "hwpx/hwpx-h-03.hwpx"),
    ("hwpx/hancom-hwp/issue_157.hwp", "hwpx/issue_157.hwpx"),
    ("hwpx/hancom-hwp/issue_241.hwp", "hwpx/issue_241.hwpx"),
    ("landscape-001.hwp", "hwpx/landscape-001.hwpx"),
    ("math-001.hwp", "hwpx/math-001.hwpx"),
    ("para-unit-01.hwp", "hwpx/para-unit-01.hwpx"),
    ("hwpx/hancom-hwp/tb-org-02.hwp", "hwpx/tb-org-02.hwpx"),
    (
        "hwpx/hancom-hwp/tbox-v-flow-01.hwp",
        "hwpx/tbox-v-flow-01.hwpx",
    ),
    (
        "issue2004_cell_image_stack.hwp",
        "issue2004_cell_image_stack.hwpx",
    ),
    (
        "issue2020/(250813) (보도자료) 2025년 7월중 가계대출 동향.hwp",
        "issue2020/(250813) (보도자료) 2025년 7월중 가계대출 동향.hwpx",
    ),
    (
        "ta-pic-001-r-쪽영역안제한.hwp",
        "ta-pic-001-r-쪽영역안제한.hwpx",
    ),
    (
        "ta-pic-001-r-쪽영역안제한no.hwp",
        "ta-pic-001-r-쪽영역안제한no.hwpx",
    ),
    ("test-image.hwp", "test-image.hwpx"),
    ("누름틀-2024.hwp", "누름틀-2024.hwpx"),
    ("대각선샘플3.hwp", "대각선샘플3.hwpx"),
    ("대각선샘플4.hwp", "대각선샘플4.hwpx"),
    ("대각선샘플5.hwp", "대각선샘플5.hwpx"),
    ("복학원서.hwp", "복학원서.hwpx"),
    ("셀보호.hwp", "셀보호.hwpx"),
    ("셀보호2.hwp", "셀보호2.hwpx"),
    ("수식-문자처럼취급-아님.hwp", "수식-문자처럼취급-아님.hwpx"),
    ("종이기준.hwp", "종이기준.hwpx"),
    ("쪽기준.hwp", "쪽기준.hwpx"),
    ("한셀OLE.hwp", "한셀OLE.hwpx"),
];

fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let hit = [
            "tail: Bytes(",
            "src: Some(Bytes(",
            "src: None",
            "xml: Some(Bytes(",
            "xml: None",
        ]
        .iter()
        .filter_map(|c| rest.find(c).map(|i| (i, *c)))
        .min();
        let Some((i, c)) = hit else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..i]);
        rest = &rest[i + c.len()..];
        if c.ends_with('(') {
            let close = if c.starts_with("tail") { ")" } else { "))" };
            let j = rest.find(close).unwrap_or(rest.len());
            rest = &rest[(j + close.len()).min(rest.len())..];
        }
    }
}

fn hash16(s: &str) -> u16 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish() as u16 | 1
}

/// Border fill references by content.
struct Fills(Vec<u16>);

impl Fills {
    fn new(info: &DocInfo) -> Fills {
        Fills(
            info.border_fills
                .iter()
                .map(|b| {
                    // A diagonal without a line keeps an arbitrary width.
                    let mut b = b.clone();
                    if b.diagonal.kind == 0 {
                        b.diagonal = BorderLine::default();
                    }
                    hash16(&strip(&format!("{b:?}")))
                })
                .collect(),
        )
    }

    fn map(&self, id: u16) -> u16 {
        if id == 0 {
            return 0;
        }
        self.0.get(id as usize - 1).copied().unwrap_or(0xFFFF)
    }
}

fn round(m: &mut Matrix) {
    for v in &mut m.0 {
        *v = (*v * 1e4).round() / 1e4;
    }
}

fn header(h: &mut ObjectHeader) {
    h.common.attr &= (1 << 28) - 1;
}

fn shape(s: &mut Shape, f: &Fills) {
    s.component.flip &= 3 | 1 << 19;
    round(&mut s.component.translation);
    for (a, b) in &mut s.component.transforms {
        round(a);
        round(b);
    }
    if let Some(st) = &mut s.component.style {
        st.reserved = None;
        if let Some(i) = &mut st.fill.image {
            i.bin_id = 0;
        }
    }
    s.parts.retain(|p| !matches!(p, ShapePart::Raw(_)));
    for p in &mut s.parts {
        match p {
            ShapePart::Child(c) => shape(c, f),
            ShapePart::Text(t) => {
                paras(&mut t.paras, f);
                t.paras.iter_mut().for_each(|p| p.line_segs.clear());
            }
            ShapePart::Picture(p) => p.image.bin_id = 0,
            ShapePart::Ole(o) => o.bin_id = 0,
            _ => {}
        }
    }
}

fn masked(t: &mut [u16]) {
    let mut i = 0;
    while i < t.len() {
        let c = t[i];
        if c < 32 && control_width(c) == 8 && i + 7 < t.len() {
            match c {
                9 => t[i + 4..i + 7].fill(0),
                4 => t[i + 2..i + 7].fill(0),
                _ => {}
            }
            i += 8;
        } else {
            i += 1;
        }
    }
}

fn paras(ps: &mut [Paragraph], f: &Fills) {
    for p in ps {
        p.header.merge_flag = p.header.merge_flag.or(Some(0));
        p.header.control_mask = 0;
        masked(&mut p.text);
        let end = p.len().saturating_sub(1);
        if p.runs.len() > 1 && p.runs.last().is_some_and(|r| r.pos == end) {
            p.runs.pop();
        }
        p.range_tags.retain(|t| t.tag >> 24 == 2);
        p.extra.clear();
        for c in &mut p.controls {
            control(c, f);
        }
    }
}

fn list(l: &mut ParaList, f: &Fills) {
    paras(&mut l.paras, f);
}

fn control(c: &mut Control, f: &Fills) {
    if let ControlKind::Raw(r) = &c.kind {
        let id = r.id;
        c.kind = ControlKind::Raw(RawControl {
            id,
            header: SharedBytes::empty(),
            children: Vec::new(),
            xml: None,
        });
    }
    match &mut c.kind {
        ControlKind::Section(s) => {
            s.children.retain(|c| !matches!(c, SectionChild::Raw(_)));
            for ch in &mut s.children {
                match ch {
                    SectionChild::PageBorder(b) => {
                        b.border_fill = f.map(b.border_fill);
                        b.attr &= 0x1F;
                    }
                    SectionChild::MasterPage(l) => list(l, f),
                    _ => {}
                }
            }
        }
        ControlKind::Table(t) => {
            header(&mut t.header);
            t.attr &= (1 << 26) - 1;
            t.border_fill = f.map(t.border_fill);
            t.extra.clear();
            if let Some(z) = &mut t.zones {
                z.iter_mut()
                    .for_each(|z| z.border_fill = f.map(z.border_fill));
            }
            if let Some(cap) = &mut t.caption {
                paras(&mut cap.paras, f);
            }
            for cell in &mut t.cells {
                cell.list_attr &= (1 << 24) - 1;
                cell.border_fill = f.map(cell.border_fill);
                paras(&mut cell.paras, f);
            }
        }
        ControlKind::Object(o) => {
            header(&mut o.header);
            o.parts.retain(|p| !matches!(p, ObjectPart::Raw(_)));
            for p in &mut o.parts {
                match p {
                    ObjectPart::Shape(s) => shape(s, f),
                    ObjectPart::Caption(c) => paras(&mut c.paras, f),
                    _ => {}
                }
            }
        }
        ControlKind::HeaderFooter(h) => list(&mut h.list, f),
        ControlKind::Note(n) => list(&mut n.list, f),
        _ => {}
    }
}

/// The model as compared: one string per table entry and per paragraph.
/// Named strings, one per compared entry.
type Entries = Vec<(String, String)>;

fn normalized(mut d: Document) -> (Entries, Entries) {
    let f = Fills::new(&d.info);
    let mut info = Vec::new();
    for (i, c) in d.info.char_shapes.iter_mut().enumerate() {
        c.faces = [0; 7];
        c.border_fill = c.border_fill.map(|b| f.map(b));
        info.push((format!("char shape {i}"), strip(&format!("{c:?}"))));
    }
    for (i, p) in d.info.para_shapes.iter_mut().enumerate() {
        p.attr2 = p.attr2.map(|a| a & !8);
        p.border_fill = f.map(p.border_fill);
        info.push((format!("para shape {i}"), strip(&format!("{p:?}"))));
    }
    for (i, t) in d.info.tab_defs.iter().enumerate() {
        info.push((format!("tab def {i}"), strip(&format!("{t:?}"))));
    }
    for (i, n) in d.info.numberings.iter().enumerate() {
        info.push((format!("numbering {i}"), strip(&format!("{n:?}"))));
    }
    for (i, s) in d.info.styles.iter().enumerate() {
        info.push((format!("style {i}"), strip(&format!("{s:?}"))));
    }
    let mut body = Vec::new();
    for (k, s) in d.sections.iter_mut().enumerate() {
        paras(&mut s.paras, &f);
        for (i, p) in s.paras.iter().enumerate() {
            body.push((
                format!("section {k} paragraph {i}"),
                strip(&format!("{p:?}")),
            ));
        }
    }
    (info, body)
}

#[test]
fn twins_read_into_equal_models() {
    let Some(root) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping twin tests");
        return;
    };
    let root = PathBuf::from(root);
    let mut failures = Vec::new();
    for (h, x) in PAIRS {
        let (hp, xp) = (root.join(h), root.join(x));
        if !hp.exists() || !xp.exists() {
            eprintln!("missing twin {h}");
            continue;
        }
        let d5 = hm_hwp5::read(SharedBytes::from(std::fs::read(&hp).unwrap())).unwrap();
        let dx = hm_hwpx::read(SharedBytes::from(std::fs::read(&xp).unwrap())).unwrap();
        let (i5, b5) = normalized(d5);
        let (ix, bx) = normalized(dx);
        let mut diffs = 0;
        for (a, b) in i5.iter().zip(&ix).chain(b5.iter().zip(&bx)) {
            if a.1 != b.1 {
                diffs += 1;
                if diffs <= 3 {
                    let (h, x) = window(&a.1, &b.1);
                    failures.push(format!("{}: {}\n  hwp : {h}\n  hwpx: {x}", name(&xp), a.0));
                }
            }
        }
        if i5.len() != ix.len() || b5.len() != bx.len() {
            failures.push(format!("{}: entry counts differ", name(&xp)));
        }
        if diffs > 3 {
            failures.push(format!("{}: {diffs} differences", name(&xp)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The two strings around their first difference.
fn window(a: &str, b: &str) -> (String, String) {
    let k = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let cut = |s: &str| {
        let mut from = k.saturating_sub(200);
        while !s.is_char_boundary(from) {
            from -= 1;
        }
        let mut to = (k + 200).min(s.len());
        while !s.is_char_boundary(to) {
            to -= 1;
        }
        s[from..to].to_string()
    };
    (cut(a), cut(b))
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

/// Prints the difference count of every pair listed in the file named by
/// `HEEMANG_TWINS` (tab-separated HWP and HWPX paths).
#[test]
#[ignore]
fn survey_twins() {
    let Some(list) = std::env::var_os("HEEMANG_TWINS") else {
        return;
    };
    for line in std::fs::read_to_string(list).unwrap().lines() {
        let Some((h, x)) = line.split_once('\t') else {
            continue;
        };
        let (Ok(a), Ok(b)) = (std::fs::read(h), std::fs::read(x)) else {
            continue;
        };
        let (Ok(d5), Ok(dx)) = (
            hm_hwp5::read(SharedBytes::from(a)),
            hm_hwpx::read(SharedBytes::from(b)),
        ) else {
            continue;
        };
        let (i5, b5) = normalized(d5);
        let (ix, bx) = normalized(dx);
        let diffs = i5
            .iter()
            .zip(&ix)
            .chain(b5.iter().zip(&bx))
            .filter(|(a, b)| a.1 != b.1)
            .count();
        let counts = i5.len() == ix.len() && b5.len() == bx.len();
        println!("{diffs}\t{counts}\t{x}");
        if let Some((a, b)) = i5
            .iter()
            .zip(&ix)
            .chain(b5.iter().zip(&bx))
            .find(|(a, b)| a.1 != b.1)
        {
            let (h, w) = window(&a.1, &b.1);
            println!("  @ {}\n  H {h}\n  X {w}", a.0);
        }
    }
}
