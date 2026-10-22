use hm_model::SharedBytes;
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(b) = std::fs::read(&path) else {
            continue;
        };
        if !hm_hwp5::is_hwp5(&b) {
            continue;
        }
        let Ok(doc) = hm_hwp5::read(SharedBytes::from(b)) else {
            continue;
        };
        println!(
            "{:?} {:x?} v{} {}",
            doc.info.compatible.as_ref().map(|c| c.target),
            doc.info.layout_compat.as_ref().map(|c| c.flags),
            doc.meta.version,
            path.rsplit('/').next().unwrap()
        );
    }
}
