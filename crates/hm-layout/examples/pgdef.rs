use hm_model::SharedBytes;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let doc = hm_hwp5::read(SharedBytes::from(std::fs::read(&path).unwrap())).unwrap();
    for (si, s) in doc.sections.iter().enumerate() {
        if let Some(pd) = s.definition().and_then(|d| d.page_def()) {
            println!(
                "s{si} attr {:#x} gutter {} body {:?}",
                pd.attr,
                pd.gutter,
                pd.body()
            );
        }
        for (pi, p) in s.paras.iter().enumerate() {
            if pi > 0
                && let Some(sd) = p.controls.iter().find_map(|c| c.as_section())
            {
                if let Some(pd) = sd.page_def() {
                    println!("  p{pi} attr {:#x} gutter {}", pd.attr, pd.gutter);
                }
            }
        }
    }
}
