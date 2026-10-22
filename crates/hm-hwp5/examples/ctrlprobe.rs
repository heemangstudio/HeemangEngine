use hm_hwp5::{cfb, deflate, record};
use hm_model::SharedBytes;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want: Vec<&str> = args[2].split(',').collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let sb = SharedBytes::from(bytes);
    let c = cfb::Cfb::open(&sb).unwrap();
    for i in 0.. {
        let Some(raw) = c.read(&format!("BodyText/Section{i}")) else {
            break;
        };
        let data = deflate::inflate(&raw, 1 << 30).unwrap_or(raw);
        let s = SharedBytes::from(data);
        let (recs, _) = record::read_records(&s);
        let mut last_text = String::new();
        for (k, r) in recs.iter().enumerate() {
            if r.tag == record::tag::PARA_TEXT {
                let u: Vec<u16> = r
                    .payload
                    .chunks(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                last_text = u
                    .iter()
                    .map(|&x| {
                        if x < 32 {
                            '·'
                        } else {
                            char::from_u32(x as u32).unwrap_or('?')
                        }
                    })
                    .collect();
            }
            if r.tag == record::tag::CTRL_HEADER && r.payload.len() >= 4 {
                let id: Vec<u8> = r.payload[0..4].iter().rev().cloned().collect();
                let ids = String::from_utf8_lossy(&id).to_string();
                if want.contains(&ids.as_str()) {
                    let hex: Vec<String> = r.payload.iter().map(|b| format!("{b:02x}")).collect();
                    println!(
                        "s{i} rec{k} lvl{} {ids} len{} [{}]",
                        r.level,
                        r.payload.len(),
                        hex.join(" ")
                    );
                    println!(
                        "   text: {}",
                        last_text.chars().take(80).collect::<String>()
                    );
                    let mut j = k + 1;
                    while j < recs.len() && recs[j].level > r.level {
                        println!(
                            "   child tag {:#x} lvl{} len{}",
                            recs[j].tag,
                            recs[j].level,
                            recs[j].payload.len()
                        );
                        j += 1;
                    }
                }
            }
        }
    }
}
