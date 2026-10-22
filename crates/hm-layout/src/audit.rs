//! Comparing our lines with the line segments Hancom stored.

use hm_model::Document;

use crate::DocLayout;

/// Agreement between our layout and the stored line segments.
#[derive(Clone, Debug, Default)]
pub struct Audit {
    /// Paragraphs that carry stored segments.
    pub paras: u32,
    /// Of those, paragraphs whose line starts all match.
    pub paras_breaks_match: u32,
    /// Paragraphs measured only with the faces the document names.
    pub exact_paras: u32,
    pub exact_breaks_match: u32,
    /// Lines compared field by field (paragraphs with matching breaks).
    pub lines: u32,
    pub height_match: u32,
    pub text_height_match: u32,
    pub baseline_match: u32,
    pub spacing_match: u32,
    pub x_match: u32,
    pub width_match: u32,
    /// Lines whose page position matches the stored `vpos`.
    pub vpos_match: u32,
    /// The first diverging paragraphs: `(section, paragraph, ours, stored)` line starts.
    pub diverging: Vec<(u32, u32, Vec<u32>, Vec<u32>)>,
}

impl Audit {
    pub fn rate(n: u32, of: u32) -> f64 {
        if of == 0 {
            100.0
        } else {
            100.0 * n as f64 / of as f64
        }
    }
}

/// Audits a layout. Only body paragraphs are compared; object contents are
/// not laid out yet.
pub fn audit(doc: &Document, layout: &DocLayout, keep: usize) -> Audit {
    let mut a = Audit::default();
    let mut vpos = std::collections::HashMap::new();
    for page in &layout.pages {
        for pl in &page.lines {
            vpos.insert((page.section, pl.para, pl.line), pl.vpos);
        }
    }
    for (si, sec) in doc.sections.iter().enumerate() {
        let Some(sl) = layout.sections.get(si) else {
            continue;
        };
        for (pi, para) in sec.paras.iter().enumerate() {
            if para.line_segs.is_empty() {
                continue;
            }
            let ours = &sl.paras[pi];
            a.paras += 1;
            let stored: Vec<u32> = para.line_segs.iter().map(|s| s.text_start).collect();
            let mine: Vec<u32> = ours.lines.iter().map(|l| l.start).collect();
            let exact = ours
                .measured
                .styles
                .iter()
                .all(|&s| layout.styles[s as usize].exact);
            a.exact_paras += u32::from(exact);
            a.exact_breaks_match += u32::from(exact && stored == mine);
            if stored != mine {
                if a.diverging.len() < keep {
                    a.diverging.push((si as u32, pi as u32, mine, stored));
                }
                continue;
            }
            a.paras_breaks_match += 1;
            for (li, (s, l)) in para.line_segs.iter().zip(&ours.lines).enumerate() {
                a.lines += 1;
                a.height_match += u32::from(s.height == l.height);
                a.text_height_match += u32::from(s.text_height == l.text_height);
                a.baseline_match += u32::from(s.baseline == l.baseline);
                a.spacing_match += u32::from(s.spacing == l.spacing);
                a.x_match += u32::from(s.col_start == l.seg_x);
                a.width_match += u32::from(s.width == l.seg_width);
                if vpos.get(&(si as u32, pi as u32, li as u32)) == Some(&s.vpos) {
                    a.vpos_match += 1;
                }
            }
        }
    }
    a
}
