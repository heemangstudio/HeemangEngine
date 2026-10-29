//! Opening and saving HWPML uses the same edit/history contract as binary HWP.
use hm_wasm::{Doc, Engine};
use serde_json::{Value, json};
fn query(doc: &mut Doc) -> Value {
    serde_json::from_str(
        &doc.query(&json!({"kind":"paragraphs","list":{"section":0}}).to_string())
            .unwrap(),
    )
    .unwrap()
}
const SAMPLE: &str = r##"<?xml version="1.0" encoding="UTF-8"?><HWPML Version="2.91"><HEAD SecCnt="1"><MAPPINGTABLE><FACENAMELIST><FONTFACE Lang="Hangul"><FONT Id="0" Name="바탕" Type="ttf"/></FONTFACE></FACENAMELIST><CHARSHAPELIST><CHARSHAPE Id="0" Height="1200" TextColor="255"><FONTID Hangul="0"/></CHARSHAPE></CHARSHAPELIST><PARASHAPELIST><PARASHAPE Id="0" Align="Center"><PARAMARGIN Left="3000" Indent="-1000" LineSpacing="160" LineSpacingType="Percent"/></PARASHAPE></PARASHAPELIST><STYLELIST><STYLE Id="0" Name="본문" Type="Para"/></STYLELIST><BINDATALIST><BINITEM Id="1" Type="Embedding" Format="png" BinData="1"/></BINDATALIST></MAPPINGTABLE></HEAD><BODY><SECTION><P ParaShape="0"><TEXT CharShape="0"><SECDEF><PAGEDEF Width="59528" Height="84188"/></SECDEF><CHAR>body😀</CHAR><HEADER Type="Both"><PARALIST><P><TEXT><CHAR>header</CHAR></TEXT></P></PARALIST></HEADER><FOOTER Type="Both"><PARALIST><P><TEXT><CHAR>footer</CHAR></TEXT></P></PARALIST></FOOTER><FOOTNOTE Number="1"><PARALIST><P><TEXT><CHAR>note</CHAR></TEXT></P></PARALIST></FOOTNOTE><ENDNOTE Number="2"><PARALIST><P><TEXT><CHAR>endnote</CHAR></TEXT></P></PARALIST></ENDNOTE><PICTURE><SHAPEOBJECT InstId="10"><SIZE Width="7200" Height="3600"/><POSITION TreatAsChar="true"/></SHAPEOBJECT><SHAPECOMPONENT OriWidth="7200" OriHeight="3600"/><IMAGE BinItem="1" Bright="5" Contrast="6" Effect="RealPic"/></PICTURE><TABLE RowCount="1" ColCount="1"><SHAPEOBJECT><SIZE Width="10000" Height="3000"/><POSITION TreatAsChar="true"/></SHAPEOBJECT><ROW><CELL RowAddr="0" ColAddr="0" RowSpan="1" ColSpan="1" Width="10000" Height="3000"><PARALIST><P><TEXT><CHAR>cell</CHAR><TABLE RowCount="1" ColCount="1"><ROW><CELL RowAddr="0" ColAddr="0" RowSpan="1" ColSpan="1"><PARALIST><P><TEXT><CHAR>nested</CHAR></TEXT></P></PARALIST></CELL></ROW></TABLE></TEXT></P></PARALIST></CELL></ROW></TABLE></TEXT></P></SECTION></BODY><TAIL><BINDATASTORAGE><BINDATA Id="1" Encoding="Base64" Compress="false">iVBORw0KGgo=</BINDATA></BINDATASTORAGE></TAIL></HWPML>"##;
#[test]
fn hml_edits_undo_and_save_without_changing_the_open_format() {
    let engine = Engine::new();
    let mut doc = engine.open(SAMPLE.as_bytes().to_vec()).unwrap();
    let before = query(&mut doc);
    let wire = before[0]["caret"][0].as_u64().unwrap();
    doc.transact(&json!({"ops":[{"op":"insertText","at":{"section":0,"para":0,"offset":wire},"text":"수정😀"}]}).to_string()).unwrap();
    let expected = query(&mut doc);
    assert_ne!(expected[0]["text"], before[0]["text"]);
    doc.undo().unwrap();
    assert_eq!(query(&mut doc)[0]["text"], before[0]["text"]);
    doc.redo().unwrap();
    for format in ["hml", "hwp5", "hwpx"] {
        let bytes = doc.serialize_as(format, false).unwrap();
        let mut reopened = engine.open(bytes).unwrap();
        assert_eq!(query(&mut reopened)[0]["text"], expected[0]["text"]);
        let state: Value = serde_json::from_str(&doc.state().unwrap()).unwrap();
        assert_eq!(state["format"], "hml");
        assert_eq!(state["dirty"], true);
    }
    let saved = doc.serialize(true).unwrap();
    let mut reopened = engine.open(saved).unwrap();
    assert_eq!(query(&mut reopened)[0]["text"], expected[0]["text"]);
    let state: Value = serde_json::from_str(&doc.state().unwrap()).unwrap();
    assert_eq!(state["dirty"], false);
}
#[test]
fn both_paired_formats_export_hml_with_their_equations() {
    fn scripts(paras: &Value) -> Vec<String> {
        paras
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["controls"].as_array().unwrap())
            .filter_map(|c| c["script"].as_str().map(str::to_owned))
            .collect()
    }
    let engine = Engine::new();
    for bytes in [
        include_bytes!("fixtures/query-equations.hwp").as_slice(),
        include_bytes!("fixtures/query-equations.hwpx"),
    ] {
        let mut doc = engine.open(bytes.to_vec()).unwrap();
        let expected = query(&mut doc);
        let expected_scripts = scripts(&expected);
        assert!(!expected_scripts.is_empty());
        let mut reopened = engine
            .open(doc.serialize_as("hml", false).unwrap())
            .unwrap();
        let actual = query(&mut reopened);
        assert_eq!(actual[0]["text"], expected[0]["text"]);
        assert_eq!(scripts(&actual), expected_scripts);
    }
}
