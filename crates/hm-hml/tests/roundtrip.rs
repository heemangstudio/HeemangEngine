use hm_model::*;
fn read(bytes: &[u8]) -> Document {
    hm_hml::read(bytes.into()).unwrap()
}
fn text(doc: &Document) -> Vec<String> {
    fn list(ps: &[Paragraph], out: &mut Vec<String>) {
        for p in ps {
            out.push(
                p.units()
                    .filter_map(|(_, u)| match u {
                        Unit::Char(c) => Some(c),
                        _ => None,
                    })
                    .collect(),
            );
            for c in &p.controls {
                match &c.kind {
                    ControlKind::Table(t) => {
                        for c in &t.cells {
                            list(&c.paras, out);
                        }
                    }
                    ControlKind::HeaderFooter(h) => list(&h.list.paras, out),
                    ControlKind::Note(n) => list(&n.list.paras, out),
                    ControlKind::Object(o) => {
                        for p in &o.parts {
                            if let ObjectPart::Shape(s) = p {
                                for part in &s.parts {
                                    if let ShapePart::Text(t) = part {
                                        list(&t.paras, out);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in &doc.sections {
        list(&s.paras, &mut out);
    }
    out
}
#[test]
fn real_samples_keep_text_tables_and_formatting_across_all_writers() {
    for bytes in [
        include_bytes!("fixtures/formatting_table.hml").as_slice(),
        include_bytes!("fixtures/aligns.hml"),
        include_bytes!("fixtures/equations.hml"),
    ] {
        let mut doc = read(bytes);
        assert_eq!(doc.meta.format, Format::Hml);
        assert!(!doc.info.char_shapes.is_empty());
        let expected = text(&doc);
        assert!(!expected.is_empty());
        // Force paragraph serialization after a realistic text edit.
        let p = &mut doc.sections[0].paras[0];
        let at = p.text.len() - 1;
        p.text.splice(at..at, "수정😀".encode_utf16());
        p.src = None;
        let expected_edited = text(&doc);
        assert_ne!(expected, expected_edited);
        let saved = hm_hml::write(&doc).unwrap();
        let again = read(&saved);
        assert_eq!(text(&again), expected_edited);
        assert_eq!(again.info.char_shapes.len(), doc.info.char_shapes.len());
        assert_eq!(again.info.para_shapes.len(), doc.info.para_shapes.len());
        for (a, b) in again.info.char_shapes.iter().zip(&doc.info.char_shapes) {
            assert_eq!(
                (a.base_size, a.color, a.attr, a.faces),
                (b.base_size, b.color, b.attr, b.faces)
            );
        }
        for (a, b) in again.info.para_shapes.iter().zip(&doc.info.para_shapes) {
            assert_eq!(
                (
                    a.indent,
                    a.margin_left,
                    a.margin_right,
                    a.space_before,
                    a.space_after,
                    a.line_spacing,
                    a.attr1
                ),
                (
                    b.indent,
                    b.margin_left,
                    b.margin_right,
                    b.space_before,
                    b.space_after,
                    b.line_spacing,
                    b.attr1
                )
            );
        }
        for (a, b) in again.info.border_fills.iter().zip(&doc.info.border_fills) {
            assert_eq!((a.attr, a.sides, a.diagonal), (b.attr, b.sides, b.diagonal));
            if let (Some(a), Some(b)) = (&a.fill, &b.fill) {
                assert_eq!(
                    (&a.solid, &a.gradient, &a.image, &a.alpha),
                    (&b.solid, &b.gradient, &b.image, &b.alpha)
                );
            }
        }
        for target in [Format::Hwp5, Format::Hwpx] {
            let mut converted = again.clone();
            converted.convert_to(target);
            let reopened = if target == Format::Hwp5 {
                hm_hwp5::read(hm_hwp5::write_new(&converted).unwrap().into()).unwrap()
            } else {
                hm_hwpx::read(hm_hwpx::write_new(&converted).unwrap().into()).unwrap()
            };
            assert_eq!(text(&reopened), expected_edited);
            let back = read(&hm_hml::write(&reopened).unwrap());
            for (a, b) in back.info.para_shapes.iter().zip(&again.info.para_shapes) {
                assert_eq!(
                    (
                        a.margin_left,
                        a.margin_right,
                        a.indent,
                        a.space_before,
                        a.space_after,
                        a.line_spacing,
                        a.attr1
                    ),
                    (
                        b.margin_left,
                        b.margin_right,
                        b.indent,
                        b.space_before,
                        b.space_after,
                        b.line_spacing,
                        b.attr1
                    ),
                    "paragraph format via {target:?}"
                );
            }
        }
    }
}
#[test]
fn headers_notes_pictures_and_nested_tables_survive_edit_and_conversion() {
    let xml = r##"<HWPML Version="2.91"><HEAD SecCnt="1"><MAPPINGTABLE><FACENAMELIST><FONTFACE Lang="Hangul"><FONT Id="0" Name="바탕" Type="ttf"/></FONTFACE></FACENAMELIST><CHARSHAPELIST><CHARSHAPE Id="0" Height="1200" TextColor="255"><FONTID Hangul="0"/></CHARSHAPE></CHARSHAPELIST><PARASHAPELIST><PARASHAPE Id="0" Align="Center"><PARAMARGIN Left="3000" Indent="-1000" LineSpacing="160" LineSpacingType="Percent"/></PARASHAPE></PARASHAPELIST><STYLELIST><STYLE Id="0" Name="본문" Type="Para"/></STYLELIST><BINDATALIST><BINITEM Id="1" Type="Embedding" Format="png" BinData="1"/></BINDATALIST></MAPPINGTABLE></HEAD><BODY><SECTION><P ParaShape="0"><TEXT CharShape="0"><SECDEF><PAGEDEF Width="59528" Height="84188"/></SECDEF><CHAR>body😀</CHAR><HEADER Type="Both"><PARALIST><P><TEXT><CHAR>header</CHAR></TEXT></P></PARALIST></HEADER><FOOTER Type="Both"><PARALIST><P><TEXT><CHAR>footer</CHAR></TEXT></P></PARALIST></FOOTER><FOOTNOTE Number="1"><PARALIST><P><TEXT><CHAR>note</CHAR></TEXT></P></PARALIST></FOOTNOTE><ENDNOTE Number="2"><PARALIST><P><TEXT><CHAR>endnote</CHAR></TEXT></P></PARALIST></ENDNOTE><PICTURE><SHAPEOBJECT InstId="10"><SIZE Width="7200" Height="3600"/><POSITION TreatAsChar="true"/></SHAPEOBJECT><SHAPECOMPONENT OriWidth="7200" OriHeight="3600"/><IMAGE BinItem="1" Bright="5" Contrast="6" Effect="RealPic"/></PICTURE><TABLE RowCount="1" ColCount="1"><SHAPEOBJECT><SIZE Width="10000" Height="3000"/><POSITION TreatAsChar="true"/></SHAPEOBJECT><ROW><CELL RowAddr="0" ColAddr="0" RowSpan="1" ColSpan="1" Width="10000" Height="3000"><PARALIST><P><TEXT><CHAR>cell</CHAR><TABLE RowCount="1" ColCount="1"><ROW><CELL RowAddr="0" ColAddr="0" RowSpan="1" ColSpan="1"><PARALIST><P><TEXT><CHAR>nested</CHAR></TEXT></P></PARALIST></CELL></ROW></TABLE></TEXT></P></PARALIST></CELL></ROW></TABLE></TEXT></P></SECTION></BODY><TAIL><BINDATASTORAGE><BINDATA Id="1" Encoding="Base64" Compress="false">iVBORw0KGgo=</BINDATA></BINDATASTORAGE></TAIL></HWPML>"##;
    let doc = read(xml.as_bytes());
    assert_eq!(
        text(&doc),
        [
            "body😀", "header", "footer", "note", "endnote", "cell", "nested"
        ]
    );
    assert_eq!(doc.info.char_shapes[0].color, 255);
    assert_eq!(doc.info.para_shapes[0].margin_left, 3000);
    assert_eq!(
        &*doc.bins.as_ref().unwrap().get(1).unwrap(),
        b"\x89PNG\r\n\x1a\n"
    );
    let saved = hm_hml::write(&doc).unwrap();
    let reopened = read(&saved);
    assert_eq!(text(&reopened), text(&doc));
    assert_eq!(reopened.info.para_shapes[0].margin_left, 3000);
    assert_eq!(
        reopened.sections[0].paras[0].controls.len(),
        doc.sections[0].paras[0].controls.len()
    );
    assert_eq!(
        &*reopened.bins.as_ref().unwrap().get(1).unwrap(),
        b"\x89PNG\r\n\x1a\n"
    );
    for target in [Format::Hwp5, Format::Hwpx] {
        let mut converted = doc.clone();
        converted.convert_to(target);
        let bytes = if target == Format::Hwp5 {
            hm_hwp5::write_new(&converted).unwrap()
        } else {
            hm_hwpx::write_new(&converted).unwrap()
        };
        let converted = if target == Format::Hwp5 {
            hm_hwp5::read(bytes.into()).unwrap()
        } else {
            hm_hwpx::read(bytes.into()).unwrap()
        };
        let hml = read(&hm_hml::write(&converted).unwrap());
        assert_eq!(text(&hml), text(&doc));
        assert_eq!(
            &*hml.bins.as_ref().unwrap().get(1).unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
    }
}
#[test]
fn detects_encodings_and_rejects_broken_documents_and_resources() {
    let source = include_bytes!("fixtures/equations.hml");
    assert!(hm_hml::is_hml(source));
    let s = String::from_utf8_lossy(source);
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(s.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert!(hm_hml::is_hml(&utf16));
    assert_eq!(text(&read(&utf16)), text(&read(source)));
    assert!(hm_hml::read(b"<HWPML><HEAD/><BODY/></HWPML>".as_slice().into()).is_err());
    assert!(hm_hml::read(b"<HWPML><HEAD></BODY>".as_slice().into()).is_err());
}

#[test]
fn legacy_korean_xml_and_compressed_resources_are_read_without_data_loss() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let original = include_bytes!("fixtures/equations.hml");
    let xml = String::from_utf8_lossy(original).replace("UTF-8", "EUC-KR");
    let (bytes, _, errors) = encoding_rs::EUC_KR.encode(&xml);
    assert!(!errors);
    assert!(hm_hml::is_hml(&bytes));
    assert_eq!(text(&read(&bytes)), text(&read(original)));
    let payload = b"binary image data\0\xff";
    let compact = STANDARD.encode(miniz_oxide::deflate::compress_to_vec(payload, 6));
    let xml = format!(
        r#"<HWPML><HEAD><MAPPINGTABLE><BINDATALIST><BINITEM Id="1" Type="Embedding" Format="png"/></BINDATALIST><PARASHAPELIST><PARASHAPE Id="0"/></PARASHAPELIST></MAPPINGTABLE></HEAD><BODY><SECTION><P><TEXT><SECDEF FirstBorder="true"/><CHAR>body</CHAR></TEXT></P></SECTION></BODY><TAIL><BINDATASTORAGE><BINDATA Id="1" Compress="true">{compact}</BINDATA></BINDATASTORAGE><SCRIPTCODE><SCRIPTSOURCE>preserved script</SCRIPTSOURCE></SCRIPTCODE></TAIL></HWPML>"#
    );
    let doc = read(xml.as_bytes());
    assert_eq!(&*doc.bins.as_ref().unwrap().get(1).unwrap(), payload);
    assert_ne!(doc.sections[0].definition().unwrap().attr & (1 << 8), 0);
    assert_eq!(doc.info.para_shapes[0].line_spacing, Some(160));
    let saved = hm_hml::write(&doc).unwrap();
    assert!(String::from_utf8_lossy(&saved).contains("preserved script"));
    assert_eq!(
        &*read(&saved).bins.as_ref().unwrap().get(1).unwrap(),
        payload
    );
    assert!(hm_hml::read(xml.replace(&compact, "broken base64").into_bytes().into()).is_err());
}
