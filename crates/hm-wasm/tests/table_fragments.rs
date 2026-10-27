//! Cell selections and repaint receipts follow physical fragment geometry.
use hm_model::{
    Cell, Control, ControlKind, ObjectCommon, ObjectHeader, PARA_END, PageDef, ParaShape,
    Paragraph, Section, SectionChild, SectionDef, Table,
};
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};

fn source() -> Vec<u8> {
    let mut doc = hm_hwpx::read(
        include_bytes!("fixtures/query-equations.hwpx")
            .to_vec()
            .into(),
    )
    .unwrap();
    doc.source = None;
    doc.info.char_shapes[0].base_size = 1000;
    doc.info.char_shapes[0].ratios = [100; 7];
    doc.info.char_shapes[0].rel_sizes = [100; 7];
    doc.info.para_shapes[0] = ParaShape {
        line_spacing_legacy: 160,
        ..Default::default()
    };
    let text = Paragraph {
        text: vec!['A' as u16, PARA_END],
        ..Default::default()
    };
    let cell = |row, col, span, height| Cell {
        row,
        col,
        row_span: span,
        col_span: 1,
        width: 3000,
        height,
        list_attr: 1 << 5,
        paras: vec![text.clone()],
        ..Default::default()
    };
    let table = Table {
        header: ObjectHeader {
            common: ObjectCommon {
                attr: (2 << 3) | (4 << 15) | (2 << 18) | (1 << 21),
                width: 9000,
                height: 6600,
                y_offset: 6200,
                ..Default::default()
            },
            ..Default::default()
        },
        attr: 2 | 8,
        rows: 3,
        cols: 3,
        padding: [100; 4],
        row_cells: vec![3, 2, 2],
        cells: vec![
            cell(0, 0, 2, 4500),
            cell(0, 1, 1, 2000),
            cell(0, 2, 1, 2000),
            cell(1, 1, 2, 4600),
            cell(1, 2, 1, 2000),
            cell(2, 0, 1, 2000),
            cell(2, 2, 1, 2000),
        ],
        ..Default::default()
    };
    let section = SectionDef {
        children: vec![SectionChild::Page(PageDef {
            width: 10000,
            height: 10000,
            ..Default::default()
        })],
        ..Default::default()
    };
    doc.sections = vec![Section {
        paras: vec![
            Paragraph {
                text: [vec![2; 8], vec!['X' as u16, PARA_END]].concat(),
                controls: vec![Control {
                    kind: ControlKind::Section(Box::new(section)),
                    src: None,
                }],
                ..Default::default()
            },
            Paragraph {
                text: [vec![11; 8], vec![PARA_END]].concat(),
                controls: vec![Control {
                    kind: ControlKind::Table(Box::new(table)),
                    src: None,
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    hm_hwpx::write_new(&doc).unwrap()
}

fn table(doc: &mut Doc) -> Value {
    serde_json::from_str(
        &doc.query(
            &json!({"kind":"table",
        "table":{"section":0,"para":1,"control":0}})
            .to_string(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn cell_rects(info: &Value, row: u16, column: u16) -> Value {
    info["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["row"] == row && c["column"] == column)
        .unwrap()["rects"]
        .clone()
}

fn select(doc: &mut Doc, row: u16, column: u16) -> Value {
    serde_json::from_str(
        &doc.select(
            &json!({"kind":"cells",
        "table":{"section":0,"para":1,"control":0},
        "from":{"row":row,"column":column},"to":{"row":row,"column":column}})
            .to_string(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn physical_cell_selection_and_continuation_repaint_survive_undo_and_save() {
    let engine = Engine::new();
    let mut doc = engine.open(source()).unwrap();
    let before = table(&mut doc);
    let closing = cell_rects(&before, 0, 2);
    assert_eq!(closing[0]["height"], 2200);
    assert_eq!(select(&mut doc, 0, 2), closing);
    let continued = cell_rects(&before, 1, 2);
    assert_eq!(continued[0]["page"], 1);
    assert_eq!(continued[0]["height"], 2300);
    assert_eq!(select(&mut doc, 1, 2), continued);

    let receipt: Value = serde_json::from_str(
        &doc.transact(
            &json!({"ops":[{
        "op":"formatParas","from":{"section":0,"para":0,"offset":8},
        "to":{"section":0,"para":0,"offset":9},"patch":{"lineSpacing":150}}]})
            .to_string(),
        )
        .unwrap(),
    )
    .unwrap();
    let after = table(&mut doc);
    let continued = cell_rects(&after, 1, 2);
    assert_ne!(continued, cell_rects(&before, 1, 2));
    assert_eq!(select(&mut doc, 1, 2), continued);
    assert!(
        receipt["changedPages"]
            .as_array()
            .unwrap()
            .contains(&json!(1))
    );
    doc.undo().unwrap();
    assert_eq!(table(&mut doc), before);
    doc.redo().unwrap();
    assert_eq!(table(&mut doc), after);
    for format in ["hwp5", "hwpx"] {
        let mut reopened = engine
            .open(doc.serialize_as(format, false).unwrap())
            .unwrap();
        assert_eq!(table(&mut reopened), after, "{format}");
        assert_eq!(select(&mut reopened, 1, 2), continued, "{format}");
    }
}
