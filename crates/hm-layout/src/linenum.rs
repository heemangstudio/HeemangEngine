//! Line numbers in the left margin (줄 번호): Hancom numbers every body line
//! of a section that shows them, empty ones included, and prints the
//! numbers right-aligned a set distance left of the body, on each line's
//! baseline, in 10 pt 함초롬바탕.

use hm_model::{LineNumbers, Script};

use crate::furniture::Mark;
use crate::measure::advance;
use crate::paginate::Page;
use crate::para::ParaLayout;
use crate::style::Styles;

const FACE: &str = "함초롬바탕";
const SIZE: i32 = 1000;

/// Numbers the lines of a section's pages. `counter` carries the last
/// number given from section to section; `first` says this is the first
/// section numbered.
pub(crate) fn number_lines(
    shape: &LineNumbers,
    paras: &[ParaLayout],
    pages: &mut [Page],
    styles: &mut Styles<'_>,
    counter: &mut u32,
    first: bool,
) {
    let start = shape.start.max(1) as u32;
    let every = shape.count_by.max(1) as u32;
    if first || shape.restart == 2 {
        *counter = start - 1;
    }
    for page in pages.iter_mut() {
        if shape.restart == 1 {
            *counter = start - 1;
        }
        let right = page.body.x - shape.distance;
        let mut marks = Vec::new();
        for l in &page.lines {
            *counter += 1;
            if *counter % every != 0 {
                continue;
            }
            let Some(line) = paras
                .get(l.para as usize)
                .and_then(|p| p.lines.get(l.line as usize))
            else {
                continue;
            };
            let text = counter.to_string();
            let mut glyphs = Vec::new();
            let mut x = 0;
            for c in text.chars() {
                let si = styles.builtin_style(FACE, Script::Latin, c, SIZE);
                let st = styles.get(si).clone();
                let face = styles.face(si).cloned();
                glyphs.push((c, si, x));
                x += advance(&st, face.as_deref(), c);
            }
            for g in &mut glyphs {
                g.2 += right - x;
            }
            marks.push(Mark {
                glyphs,
                baseline: l.y + line.baseline,
            });
        }
        page.marks.extend(marks);
    }
}
