use hm_hwp5::{cfb, deflate, record};
use hm_model::SharedBytes;
fn dump(name: &str, data: Vec<u8>) {
    let s = SharedBytes::from(data);
    let (recs, _) = record::read_records(&s);
    for r in recs.iter() {
        let hex: String = r
            .payload
            .iter()
            .take(96)
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut extra = String::new();
        if r.tag == record::tag::PARA_TEXT {
            let u: Vec<u16> = r
                .payload
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            extra = u
                .iter()
                .map(|&x| {
                    if x < 32 {
                        format!("<{x}>")
                    } else {
                        char::from_u32(x as u32).unwrap_or('?').to_string()
                    }
                })
                .collect();
        }
        println!(
            "{name} tag {:#x} lvl{} len{} [{}] {}",
            r.tag,
            r.level,
            r.payload.len(),
            hex,
            extra
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let sb = SharedBytes::from(bytes);
    let c = cfb::Cfb::open(&sb).unwrap();
    let fh = c.read("FileHeader").unwrap();
    println!(
        "fileheader version {:02x?} flags {:08x}",
        &fh[32..36],
        u32::from_le_bytes(fh[36..40].try_into().unwrap())
    );
    let di = c.read("DocInfo").unwrap();
    dump("docinfo", deflate::inflate(&di, 1 << 30).unwrap_or(di));
    for i in 0.. {
        let Some(raw) = c.read(&format!("BodyText/Section{i}")) else {
            break;
        };
        dump("body", deflate::inflate(&raw, 1 << 30).unwrap_or(raw));
    }
}
