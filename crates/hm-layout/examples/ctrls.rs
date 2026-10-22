use hm_model::{ControlKind, SectionChild, SharedBytes};
fn hex(b: &[u8]) -> String {
    b.iter()
        .take(48)
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&a[1]).unwrap())).unwrap();
    let want: Vec<&str> = a.get(2).map(|s| s.split(',').collect()).unwrap_or_default();
    for (si, sec) in doc.sections.iter().enumerate() {
        for (pi, p) in sec.paras.iter().enumerate() {
            for c in &p.controls {
                let id = format!("{:?}", c.id());
                if !want.is_empty() && !want.iter().any(|w| id.contains(w)) {
                    continue;
                }
                match &c.kind {
                    ControlKind::Raw(r) => {
                        println!(
                            "s{si} p{pi} {id} hdr[{}] {}",
                            r.header.len(),
                            hex(&r.header)
                        );
                        for ch in r.children.iter().take(12) {
                            println!(
                                "    tag {:#x} lvl {} len {} {}",
                                ch.tag,
                                ch.level,
                                ch.payload.len(),
                                hex(&ch.payload)
                            );
                        }
                    }
                    ControlKind::Section(s) => {
                        println!("s{si} p{pi} secd attr {:#x} pgno {}", s.attr, s.page_number);
                        for ch in &s.children {
                            match ch {
                                SectionChild::Raw(r) => println!(
                                    "    raw tag {:#x} lvl {} len {} {}",
                                    r.tag,
                                    r.level,
                                    r.payload.len(),
                                    hex(&r.payload)
                                ),
                                SectionChild::PageBorder(b) => {
                                    println!("    border {:?}", (b.attr, b.offsets, b.border_fill))
                                }
                                SectionChild::NoteShape(n) => println!(
                                    "    note {:#x} sep {} {} {} {} line {} {} {:#x}",
                                    n.attr,
                                    n.separator_length,
                                    n.space_above,
                                    n.space_below,
                                    n.space_between,
                                    n.line_kind,
                                    n.line_width,
                                    n.line_color
                                ),
                                SectionChild::MasterPage(l) => println!(
                                    "    master {} paras attr {:#x}",
                                    l.paras.len(),
                                    l.attr
                                ),
                                SectionChild::Page(pd) => println!(
                                    "    page {:?}",
                                    (
                                        pd.width,
                                        pd.height,
                                        pd.margin_left,
                                        pd.margin_right,
                                        pd.margin_top,
                                        pd.margin_bottom,
                                        pd.margin_header,
                                        pd.margin_footer,
                                        pd.gutter,
                                        pd.attr
                                    )
                                ),
                            }
                        }
                    }
                    ControlKind::Columns(c) => println!(
                        "s{si} p{pi} cold attr {:#x} kind {} n {} gap {} cols {:?} line {} {} brk {:#x}",
                        c.attr,
                        c.kind(),
                        c.count(),
                        c.gap,
                        c.columns,
                        c.line_kind,
                        c.line_width,
                        p.header.break_kind
                    ),
                    _ => println!("s{si} p{pi} {id}"),
                }
            }
        }
    }
}
