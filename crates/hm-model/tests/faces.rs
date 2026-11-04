//! Adding a face to a script slot keeps what existing text draws.
use hm_model::{CharShape, DocInfo, FaceName, HwpString};

fn face(name: &str) -> FaceName {
    FaceName {
        name: HwpString::from_text(name),
        ..FaceName::default()
    }
}

/// Found in the round trip of formatting-20261104/C (`d-rowbreak-midpage-declared-fi`): a
/// document with only a Hangul face list draws its Latin text with the Hangul face; a new Latin
/// face must not land on the index that text names.
#[test]
fn a_new_face_does_not_take_an_index_existing_text_names() {
    let mut info = DocInfo::default();
    info.faces[0] = vec![face("함초롬바탕")];
    info.char_shapes.push(CharShape::default());
    let id = info.face_index(1, &face("맑은 고딕"));
    assert_eq!(id, 1);
    let names: Vec<String> = info.faces[1]
        .iter()
        .map(|f| f.name.to_string_lossy())
        .collect();
    assert_eq!(names, ["함초롬바탕", "맑은 고딕"]);
    assert_eq!(info.face_index(1, &face("맑은 고딕")), 1);
}
