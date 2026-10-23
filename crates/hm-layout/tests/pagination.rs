//! Page counts against Hancom Office for Mac's PDF exports of the corpus.
//!
//! Needs `HEEMANG_CORPUS` (documents) and `HEEMANG_FONTS` (`:`-separated
//! font files or directories, walked recursively); skips when either is
//! unset. The expected counts live in `data/hancom_pages.tsv` (every
//! corpus document Hancom exported, with the old engine's count). The ratchet
//! holds for Hancom's TTF and HFT fonts with its code tables:
//!
//! ```sh
//! H="/Applications/Hancom Office HWP.app/Contents/Resources/Hnc"
//! HEEMANG_CORPUS=<heemang-cleanroom>/fixtures/samples \
//! HEEMANG_FONTS="$H/Shared/TTF:$H/Shared/Fonts:$H/Bin/Code" \
//! cargo test --profile parity -p hm-layout --test pagination -- --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hm_fonts::FontSystem;
use hm_model::SharedBytes;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn fonts(spec: &str) -> FontSystem {
    let mut fs = FontSystem::new();
    let mut files = Vec::new();
    for part in spec.split(':').filter(|s| !s.is_empty()) {
        walk(Path::new(part), &mut files);
    }
    files.sort();
    // Hancom's HFT catalog tells the styles of HFT files apart, so it goes
    // in before them.
    if let Some(info) = files
        .iter()
        .find(|f| f.file_name().is_some_and(|n| n == "hftinfo.dat"))
        && let Ok(bytes) = std::fs::read(info)
    {
        fs.add_hft_info(&bytes);
    }
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "hft") {
            fs.add(f.display().to_string(), Arc::new(bytes));
        } else if name.starts_with("u300_") {
            fs.add_code_table(&bytes);
        } else if name == "FontMap.dat" {
            fs.add_font_map(&bytes);
        }
    }
    fs
}

#[test]
fn page_counts_match_hancom() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let fonts = fonts(&font_spec);
    let table = include_str!("data/hancom_pages.tsv");
    let (mut checked, mut equal, mut old_equal) = (0, 0, 0);
    let mut behind = Vec::new();
    for line in table.lines().filter(|l| !l.starts_with('#')) {
        let mut cols = line.split('\t');
        let (Some(path), Some(pages)) = (cols.next(), cols.next()) else {
            continue;
        };
        let pages: usize = pages.trim().parse().unwrap();
        let old: Option<usize> = cols.next().and_then(|o| o.trim().parse().ok());
        let Ok(bytes) = std::fs::read(Path::new(&corpus).join(path)) else {
            continue;
        };
        let doc = if hm_hwp5::is_hwp5(&bytes) {
            hm_hwp5::read(SharedBytes::from(bytes)).ok()
        } else if hm_hwpx::is_hwpx(&bytes) {
            hm_hwpx::read(SharedBytes::from(bytes)).ok()
        } else {
            None
        };
        let Some(doc) = doc else { continue };
        let ours = hm_layout::layout(&doc, &fonts).pages.len();
        checked += 1;
        old_equal += usize::from(old == Some(pages));
        if ours == pages {
            equal += 1;
        } else {
            let old = old.map_or(String::from("-"), |o| o.to_string());
            eprintln!("{ours:>4} pages, Hancom {pages:>4}, old engine {old:>4}: {path}");
            if old == pages.to_string() {
                behind.push(path);
            }
        }
    }
    eprintln!("page counts equal to Hancom: {equal} of {checked} (old engine {old_equal})");
    eprintln!("behind the old engine on {}: {behind:?}", behind.len());
    // A ratchet: raise it as pagination improves.
    assert!(
        equal >= RATCHET,
        "{equal} of {checked} page counts match, below {RATCHET}"
    );
}

const RATCHET: usize = 647;

/// Hancom breaks a line holding an automatic page number with the number it
/// stored, so every page of `probes/page-number-ladder.hwpx` (a ladder of
/// paragraphs ending in the page number, one page each, 101 pages) breaks
/// its paragraphs the same way while showing its own number.
#[test]
fn page_numbers_in_text_keep_the_stored_line_breaks() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let path = Path::new(&corpus).join("../probes/page-number-ladder.hwpx");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} is missing; skipping", path.display());
        return;
    };
    let doc = hm_hwpx::read(SharedBytes::from(bytes)).unwrap();
    let layout = hm_layout::layout(&doc, &fonts(&font_spec));
    assert_eq!(layout.pages.len(), 101);
    let paras = &doc.sections[0].paras;
    let mut first: Option<Vec<usize>> = None;
    for (k, page) in layout.pages.iter().enumerate() {
        // Lines per paragraph on the page, in order.
        let mut counts: Vec<(u32, usize)> = Vec::new();
        for l in &page.lines {
            match counts.last_mut() {
                Some((p, n)) if *p == l.para => *n += 1,
                _ => counts.push((l.para, 1)),
            }
        }
        let lines: Vec<usize> = counts.iter().map(|c| c.1).collect();
        assert_eq!(lines, *first.get_or_insert(lines.clone()), "page {}", k + 1);
        // The number shown is the page's.
        let pi = counts[0].0 as usize;
        let pl = layout.para(0, pi as u32);
        let slot = paras[pi]
            .units()
            .find_map(|(pos, u)| match u {
                hm_model::Unit::Extended { index, .. } => matches!(
                    paras[pi].controls[index as usize].kind,
                    hm_model::ControlKind::AutoNumber(_)
                )
                .then_some(pos),
                _ => None,
            })
            .unwrap();
        let shown: String = pl
            .measured
            .items
            .iter()
            .filter(|(pos, _)| (slot..slot + 8).contains(pos))
            .filter_map(|(_, kind)| match kind {
                hm_layout::Kind::Char(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(shown, (k + 1).to_string());
    }
}

/// Hancom moves an empty paragraph that does not fit at a page foot to the
/// next page, where it takes its line before the following text, when the
/// section does not hide empty lines. `probes/empty-paragraph-foot.hwpx`
/// fills twelve pages to within 1122 down to -638 HU of the bottom before an
/// empty paragraph and a "NEXT" paragraph; Hancom's PDF shows where each
/// "NEXT" starts on the following page.
#[test]
fn empty_paragraphs_that_do_not_fit_move_to_the_next_page() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let path = Path::new(&corpus).join("../probes/empty-paragraph-foot.hwpx");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} is missing; skipping", path.display());
        return;
    };
    let doc = hm_hwpx::read(SharedBytes::from(bytes)).unwrap();
    let layout = hm_layout::layout(&doc, &fonts(&font_spec));
    let paras = &doc.sections[0].paras;
    let mut next_tops = Vec::new();
    for page in &layout.pages {
        for l in &page.lines {
            let text: String = paras[l.para as usize]
                .units()
                .filter_map(|(_, u)| match u {
                    hm_model::Unit::Char(c) => Some(c),
                    _ => None,
                })
                .collect();
            if text.starts_with("NEXT") {
                next_tops.push(l.vpos);
            }
        }
    }
    // Hancom's tops: the first case fits its empty paragraph, the next ten
    // move it, and the last also moves the filler's final line.
    let mut hancom = vec![1600; 12];
    (hancom[0], hancom[11]) = (0, 3200);
    assert_eq!(next_tops, hancom);
}

/// A page takes its even or odd page border record when that record draws
/// something and the record for both kinds otherwise, and bit 0 of the
/// record measures the offsets from the body (set) or the paper edges.
/// `probes/page-border-kinds-a.hwpx` stores a thin box for both kinds, an
/// empty even record and a red odd box; `-b` an empty record for both, a thin
/// even box and a red odd box. Their offsets tell the records apart (both
/// 1417, even 2834, odd 4251, all from the body), as Hancom's PDFs show.
/// The sample `종이기준.hwp` draws its box 1417 HU inside the paper edges.
#[test]
fn page_borders_take_the_record_of_their_page_kind_that_draws() {
    let Some(corpus) = std::env::var_os("HEEMANG_CORPUS") else {
        eprintln!("HEEMANG_CORPUS is not set; skipping");
        return;
    };
    let fonts = FontSystem::new();
    let lefts = |path: &Path| -> Option<Vec<i32>> {
        let bytes = std::fs::read(path).ok()?;
        let doc = if path.extension().is_some_and(|e| e == "hwpx") {
            hm_hwpx::read(SharedBytes::from(bytes)).unwrap()
        } else {
            hm_hwp5::read(SharedBytes::from(bytes)).unwrap()
        };
        let layout = hm_layout::layout(&doc, &fonts);
        Some(
            layout
                .pages
                .iter()
                .map(|p| p.border.map_or(-1, |b| b.rect.x - p.body.x))
                .collect(),
        )
    };
    let corpus = Path::new(&corpus);
    let cases = [
        (
            "../probes/page-border-kinds-a.hwpx",
            vec![-4251, -1417, -4251, -1417],
        ),
        (
            "../probes/page-border-kinds-b.hwpx",
            vec![-4251, -2834, -4251, -2834],
        ),
    ];
    for (name, want) in cases {
        match lefts(&corpus.join(name)) {
            Some(got) => assert_eq!(got, want, "{name}"),
            None => eprintln!("{name} is missing; skipping"),
        }
    }
    let paper = corpus.join("종이기준.hwp");
    if let Ok(bytes) = std::fs::read(&paper) {
        let doc = hm_hwp5::read(SharedBytes::from(bytes)).unwrap();
        let layout = hm_layout::layout(&doc, &fonts);
        let b = layout.pages[0].border.expect("a border");
        assert_eq!((b.rect.x, b.rect.y), (1417, 1417));
    }
}

/// A column region that starts below another on the same page begins the
/// section's region gap (here 1134) below the bottom of the last line, its
/// line spacing left out. `probes/column-region-gap.hwpx` alternates one-
/// and two-column regions of one line each in several faces and sizes;
/// Hancom's PDF puts each line that far below the last, within its baseline
/// rounding.
#[test]
fn column_regions_start_one_region_gap_below_the_last_line() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let path = Path::new(&corpus).join("../probes/column-region-gap.hwpx");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} is missing; skipping", path.display());
        return;
    };
    let doc = hm_hwpx::read(SharedBytes::from(bytes)).unwrap();
    let layout = hm_layout::layout(&doc, &fonts(&font_spec));
    let lines = &layout.pages[0].lines;
    assert!(lines.len() > 10);
    for w in lines.windows(2) {
        let prev = &layout.para(0, w[0].para).lines[w[0].line as usize];
        assert_eq!(w[1].vpos - w[0].vpos, prev.height + 1134, "{:?}", w[1]);
    }
}

/// An odd/even page adjustment numbers its page with the next number of
/// that parity: `probes/page-parity-odd.hwpx` asks for an odd page on the
/// page numbered 2, which Hancom numbers 3, and the pages after it 4 and 5.
#[test]
fn odd_even_page_adjustment_renumbers_its_page() {
    let (Some(corpus), Some(font_spec)) = (
        std::env::var_os("HEEMANG_CORPUS"),
        std::env::var("HEEMANG_FONTS").ok(),
    ) else {
        eprintln!("HEEMANG_CORPUS or HEEMANG_FONTS is not set; skipping");
        return;
    };
    let path = Path::new(&corpus).join("../probes/page-parity-odd.hwpx");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("{} is missing; skipping", path.display());
        return;
    };
    let doc = hm_hwpx::read(SharedBytes::from(bytes)).unwrap();
    let layout = hm_layout::layout(&doc, &fonts(&font_spec));
    let numbers: Vec<u32> = layout.pages.iter().map(|p| p.number).collect();
    assert_eq!(numbers, [1, 1, 3, 4, 5]);
}
