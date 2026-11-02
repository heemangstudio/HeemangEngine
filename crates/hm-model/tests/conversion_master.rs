use hm_model::*;

#[test]
fn last_page_master_sources_are_sanitized_only_for_a_format_change() {
    let bytes = SharedBytes::from(vec![1, 2, 3]);
    let leaf = Paragraph {
        text: vec![b'a' as u16, PARA_END],
        runs: vec![CharRun { pos: 0, shape: 7 }],
        line_segs: vec![LineSeg {
            width: 43000,
            height: 1600,
            ..Default::default()
        }],
        src: Some(bytes.clone()),
        header: ParaHeader {
            tail: bytes.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut parent = leaf.clone();
    parent.controls.push(Control {
        src: Some(bytes.clone()),
        kind: ControlKind::Note(Box::new(Note {
            tail: bytes.clone(),
            list: ParaList {
                tail: bytes.clone(),
                paras: vec![leaf.clone()],
                ..Default::default()
            },
            ..Default::default()
        })),
    });
    let mut doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0),
            flags: 0,
        },
        info: DocInfo::default(),
        sections: vec![Section {
            last_page_master: Some(ParaList {
                tail: bytes.clone(),
                paras: vec![parent],
                ..Default::default()
            }),
            ..Default::default()
        }],
        source: Some(bytes),
        bins: None,
        diagnostics: vec![],
    };
    let individual = doc.sections[0].last_page_master.as_ref().unwrap().clone();
    doc.sections[0].page_masters.push((2, individual));
    let before = format!("{doc:?}");
    doc.convert_to(Format::Hwpx);
    assert_eq!(format!("{doc:?}"), before);
    doc.convert_to(Format::Hwp5);
    for master in [
        doc.sections[0].last_page_master.as_ref().unwrap(),
        &doc.sections[0].page_masters[0].1,
    ] {
        assert!(master.tail.is_empty());
        let parent = &master.paras[0];
        assert!(parent.src.is_none());
        assert!(parent.header.tail.is_empty());
        assert!(parent.controls[0].src.is_none());
        let ControlKind::Note(note) = &parent.controls[0].kind else {
            panic!("note")
        };
        assert!(note.tail.is_empty());
        assert!(note.list.tail.is_empty());
        assert!(note.list.paras[0].src.is_none());
        assert!(note.list.paras[0].header.tail.is_empty());
        for p in [parent, &note.list.paras[0]] {
            assert_eq!(p.text, leaf.text);
            assert_eq!(p.runs, leaf.runs);
            assert_eq!(p.line_segs, leaf.line_segs);
        }
    }
}

#[test]
fn hwp5_gets_the_wraps_and_target_hancom_reads_in_hwpx() {
    let object = |wrap: u32| Control {
        src: None,
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: (wrap << 21) | 1,
                    ..Default::default()
                },
                ..Default::default()
            },
            parts: Vec::new(),
        })),
    };
    let wraps = |doc: &Document| -> Vec<u32> {
        let outer = &doc.sections[0].paras[0].controls;
        outer
            .iter()
            .map(|c| c.object_common().unwrap().attr)
            .collect()
    };
    let doc = |lists_layout: bool| Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0),
            flags: 0,
        },
        info: DocInfo {
            compatible: Some(CompatibleDocument {
                target: 2,
                ..Default::default()
            }),
            layout_compat: lists_layout.then(LayoutCompatibility::default),
            ..Default::default()
        },
        sections: vec![Section {
            paras: vec![Paragraph {
                text: vec![PARA_END],
                controls: vec![
                    object(4),
                    object(5),
                    object(0),
                    object(1),
                    object(2),
                    object(3),
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
        source: None,
        bins: None,
        diagnostics: vec![],
    };
    let mut a = doc(false);
    a.convert_to(Format::Hwp5);
    assert_eq!(
        wraps(&a),
        [1, 1, 1, (1 << 21) | 1, (2 << 21) | 1, (3 << 21) | 1]
    );
    assert_eq!(a.info.compatible.as_ref().unwrap().target, 0);
    let mut b = doc(true);
    b.convert_to(Format::Hwp5);
    assert_eq!(b.info.compatible.as_ref().unwrap().target, 2);
}

#[test]
fn the_print_record_reads_as_hancom_wrote_it_and_writes_back() {
    // Hancom's conversion of an HWPX printing two pages on a sheet.
    let hex = "1c02010000000702008007020800000006400600040000000e400600000000000a400600000000001f400700640000001d400600000000001a4006000000000010400700000000002040070064000000";
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|k| u8::from_str_radix(&hex[k * 2..k * 2 + 2], 16).unwrap())
        .collect();
    let print = PrintInfo::from_doc_data(&bytes).unwrap();
    assert_eq!(print.method, 4);
    assert_eq!((print.zoom_x, print.zoom_y), (100, 100));
    assert_eq!(print.overlap, Some(0));
    assert_eq!(print.to_doc_data(), bytes);
    assert_eq!(PrintInfo::from_doc_data(&bytes[..bytes.len() - 1]), None);
}

#[test]
fn hwp5_stores_each_heading_level_where_hancom_reads_it() {
    let shape = |level: u32, full: bool| ParaShape {
        attr1: (1 << 23) | (level << 25),
        attr2: full.then_some(0),
        attr3: full.then_some(0),
        line_spacing: full.then_some(160),
        ..Default::default()
    };
    let mut doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0),
            flags: 0,
        },
        info: DocInfo {
            para_shapes: vec![shape(1, true), shape(0, true), shape(1, false)],
            ..Default::default()
        },
        sections: Vec::new(),
        source: None,
        bins: None,
        diagnostics: vec![],
    };
    doc.convert_to(Format::Hwp5);
    let tails: Vec<&[u8]> = doc.info.para_shapes.iter().map(|p| &*p.tail).collect();
    assert_eq!(tails, [&[1u8, 0, 0, 0][..], &[0, 0, 0, 0], &[]]);
}

#[test]
fn a_mark_named_by_its_command_is_the_same_proofreading_mark() {
    let field = |id: &[u8; 4], command: &str| Field {
        id: CtrlId::from_chars(id),
        command: HwpString::from_text(command),
        ..Default::default()
    };
    assert_eq!(
        field(b"%unk", "$RevisionDelete;").proofreading(),
        Some("DELETE")
    );
    assert_eq!(
        field(b"%unk", "$RevisionSign;1;").proofreading(),
        Some("SIGN")
    );
    assert_eq!(
        field(b"%%*d", "$RevisionDelete;").proofreading(),
        Some("DELETE")
    );
    assert_eq!(
        field(b"%unk", "MEMO/65535/4/1/2/USER/;").proofreading(),
        None
    );
    assert_eq!(field(b"%unk", "$RevisionNoSuchMark;").proofreading(), None);
}
