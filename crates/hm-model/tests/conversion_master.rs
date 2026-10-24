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
    let before = format!("{doc:?}");
    doc.convert_to(Format::Hwpx);
    assert_eq!(format!("{doc:?}"), before);
    doc.convert_to(Format::Hwp5);
    let master = doc.sections[0].last_page_master.as_ref().unwrap();
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
