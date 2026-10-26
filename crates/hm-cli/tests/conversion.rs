//! CLI conversion preserves furniture that the current pages do not paint.
use std::process::Command;

#[test]
fn inactive_and_superseded_furniture_survives_cross_format_save() {
    let furniture = |kind: &str, side: &str, text: &str| {
        format!(
            r#"<hp:ctrl><hp:{kind} applyPageType="{side}"><hp:subList textWidth="4000" textHeight="1000"><hp:p><hp:run charPrIDRef="0"><hp:t>{text}</hp:t></hp:run></hp:p></hp:subList></hp:{kind}></hp:ctrl>"#
        )
    };
    let section = format!(
        r#"<hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:p><hp:run charPrIDRef="0"><hp:secPr><hp:pagePr width="4000" height="12000"><hp:margin left="0" right="0" top="0" bottom="0" header="1000" footer="1000" gutter="0"/></hp:pagePr></hp:secPr>{}{}{}{}<hp:t>body</hp:t></hp:run></hp:p></hs:sec>"#,
        furniture("header", "EVEN", "even header"),
        furniture("footer", "EVEN", "even footer"),
        furniture("header", "ODD", "superseded header"),
        furniture("header", "ODD", "active header"),
    );
    let parts = [
        ("mimetype", b"application/hwp+zip".as_slice()),
        ("Contents/content.hpf", br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="header" href="Contents/header.xml"/><opf:item id="section0" href="Contents/section0.xml"/></opf:manifest><opf:spine><opf:itemref idref="section0"/></opf:spine></opf:package>"#.as_slice()),
        ("Contents/header.xml", br#"<hh:head version="1.5" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head"><hh:refList><hh:charProperties itemCnt="1"><hh:charPr id="0" height="1000"/></hh:charProperties><hh:paraProperties itemCnt="1"><hh:paraPr id="0"/></hh:paraProperties></hh:refList></hh:head>"#.as_slice()),
        ("Contents/section0.xml", section.as_bytes()),
    ];
    let bytes = hm_hwpx::zip::write(
        None,
        &parts
            .iter()
            .map(|(name, data)| hm_hwpx::zip::Member::New {
                name,
                data,
                stored: true,
            })
            .collect::<Vec<_>>(),
    );
    let dir = std::env::temp_dir().join(format!(
        "heemang-cli-conversion-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.hwpx");
    std::fs::write(&source, &bytes).unwrap();
    let mut input = source.clone();
    for ext in ["hwp", "hwpx"] {
        let output = dir.join(format!("converted.{ext}"));
        let result = Command::new(env!("CARGO_BIN_EXE_heemang"))
            .arg("save")
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let written = std::fs::read(&output).unwrap();
        let reopened = if ext == "hwp" {
            hm_hwp5::read(written.into()).unwrap()
        } else {
            hm_hwpx::read(written.into()).unwrap()
        };
        assert_eq!(
            hm_layout::layout(&reopened, &hm_fonts::FontSystem::new())
                .pages
                .len(),
            1,
            "conversion must not add a page to display inactive furniture"
        );
        let furniture: Vec<_> = reopened.sections[0].paras[0]
            .controls
            .iter()
            .filter_map(|c| c.as_header_footer())
            .map(|h| {
                (
                    h.footer,
                    h.side(),
                    String::from_utf16_lossy(&h.list.paras[0].text),
                )
            })
            .collect();
        assert_eq!(
            furniture
                .iter()
                .map(|h| h.2.trim_end_matches('\r'))
                .collect::<Vec<_>>(),
            [
                "even header",
                "even footer",
                "superseded header",
                "active header"
            ]
        );
        assert!(furniture[1].0);
        assert_eq!(furniture[0].1, hm_model::PageSide::Even);
        assert_eq!(furniture[3].1, hm_model::PageSide::Odd);
        input = output;
    }
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    std::fs::remove_dir_all(dir).unwrap();
}
