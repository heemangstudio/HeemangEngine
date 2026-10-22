//! The one line of text a form control shows: a button's caption, an edit
//! box's entry or a combo box's selection, set in the control's char shape.

use hm_model::{FormObject, Script};

use crate::measure::{Blank, advance, blank_advance};
use crate::script::script_of;
use crate::style::Styles;

#[derive(Clone, Debug, Default)]
pub struct FormText {
    /// Character, style index and pen x from the text's start, in HU.
    pub glyphs: Vec<(char, u32, i32)>,
    pub width: i32,
    /// The char shape's base size.
    pub size: i32,
    /// Ascent less descent of the first face at `size`: the baseline sits
    /// half of it below the text's vertical center.
    pub rise: i32,
}

pub fn measure(form: &FormObject, styles: &mut Styles<'_>) -> FormText {
    let cs_id = form.char_shape();
    let cs = styles.char_shape_ref(cs_id).cloned().unwrap_or_default();
    let size = if cs.base_size > 0 { cs.base_size } else { 1000 };
    let mut out = FormText {
        size,
        ..Default::default()
    };
    let mut script = Script::Hangul;
    let mut first = None;
    for ch in form.display_text().chars() {
        let (adv, si) = match script_of(ch) {
            Some(s) => {
                script = s;
                let si = styles.style_for(cs_id, s, Some(ch));
                let face = styles.face(si).cloned();
                first.get_or_insert(si);
                (advance(styles.get(si), face.as_deref(), ch), si)
            }
            // Blanks take the face's own advance, unlike body text.
            None => {
                let si = styles.style_for(cs_id, script, Some(ch));
                let face = styles.face(si).cloned();
                match face.as_deref().filter(|f| f.has(ch)) {
                    Some(f) => (advance(styles.get(si), Some(f), ch), si),
                    None => (blank_advance(&cs, Blank::Ordinary), si),
                }
            }
        };
        out.glyphs.push((ch, si, out.width));
        out.width += adv;
    }
    let si = first.unwrap_or_else(|| styles.style_for(cs_id, Script::Hangul, None));
    out.rise = match styles.face(si) {
        Some(f) => (f.ascent - f.descent) as i64 * size as i64 / f.units_per_em.max(1) as i64,
        None => size as i64 * 6 / 10,
    } as i32;
    out
}
