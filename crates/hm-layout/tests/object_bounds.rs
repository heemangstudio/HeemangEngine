use hm_layout::FrameKind;
use hm_model::*;

fn paragraph(controls: Vec<Control>) -> Paragraph {
    let mut text = Vec::new();
    for c in &controls {
        let code = match &c.kind {
            ControlKind::Section(_) => 2,
            ControlKind::HeaderFooter(_) => 16,
            _ => 11,
        };
        text.extend([code, 0, 0, 0, 0, 0, 0, code]);
    }
    text.push(PARA_END);
    Paragraph {
        text,
        controls,
        runs: vec![CharRun { pos: 0, shape: 0 }],
        ..Default::default()
    }
}

fn floating(offset: i32, constrained: bool) -> Control {
    Control {
        kind: ControlKind::Object(Box::new(ObjectControl {
            id: CtrlId::SHAPE,
            header: ObjectHeader {
                common: ObjectCommon {
                    attr: (2 << 3)
                        | (2 << 8)
                        | (4 << 15)
                        | (2 << 18)
                        | (3 << 21)
                        | if constrained { 1 << 13 } else { 0 },
                    y_offset: offset,
                    width: 500,
                    height: 500,
                    ..Default::default()
                },
                ..Default::default()
            },
            parts: Vec::new(),
        })),
        src: None,
    }
}

#[test]
fn body_constrained_objects_stay_below_the_body_top_without_moving_header_objects() {
    let section = Control {
        kind: ControlKind::Section(Box::new(SectionDef {
            children: vec![SectionChild::Page(PageDef {
                width: 12000,
                height: 16000,
                margin_top: 2000,
                margin_header: 1000,
                ..Default::default()
            })],
            ..Default::default()
        })),
        src: None,
    };
    let header = Control {
        kind: ControlKind::HeaderFooter(Box::new(HeaderFooter {
            list: ParaList {
                paras: vec![paragraph(vec![floating(-400, true)])],
                ..Default::default()
            },
            ..Default::default()
        })),
        src: None,
    };
    let doc = Document {
        meta: DocMeta {
            format: Format::Hwpx,
            version: Version(0),
            flags: 0,
        },
        info: DocInfo {
            char_shapes: vec![CharShape {
                base_size: 1000,
                ratios: [100; 7],
                ..Default::default()
            }],
            para_shapes: vec![ParaShape::default()],
            ..Default::default()
        },
        sections: vec![Section {
            paras: vec![paragraph(vec![
                section,
                header,
                floating(-500, true),
                floating(-700, false),
                floating(300, true),
            ])],
            ..Default::default()
        }],
        source: None,
        bins: None,
        diagnostics: Vec::new(),
    };
    let layout = hm_layout::layout(&doc, &hm_fonts::FontSystem::new());
    assert_eq!(layout.pages.len(), 1);
    let page = &layout.pages[0];
    let object = |control| page.objects.iter().find(|o| o.control == control).unwrap();
    assert_eq!(object(2).rect.y, page.body.y);
    assert!(object(3).rect.y < page.body.y);
    assert_eq!(object(4).rect.y, page.body.y + 300);
    let header = page
        .frames
        .iter()
        .find(|f| f.kind == FrameKind::Header)
        .unwrap();
    assert!(header.objects[0].rect.y < header.rect.y);
}
