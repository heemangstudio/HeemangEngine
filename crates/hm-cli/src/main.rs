//! `heemang`: inspect, lay out, render and save HWP documents.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use hm_fonts::FontSystem;
use hm_layout::{Audit, DocLayout};
use hm_model::{Document, SharedBytes};

const USAGE: &str = "usage:
  heemang info <doc> [--font-path P]...
  heemang export-png <doc> -o <dir> [--dpi N] [-p <page>] [--font-path P]...
  heemang export-svg <doc> -o <dir> [-p <page>] [--dpi N] [--font-path P]...
  heemang export-pdf <doc> -o <file.pdf> [-p <page>] [--dpi N] [--font-path P]...
  (vector text and rules; pictures and brushes rendered at --dpi, 200 by
  default for these two)
  heemang export-text <doc> [-o <file>]
  heemang export-markdown <doc> [-o <file>]
  heemang layout-dump <doc> [-p <page>] [--audit] [--font-path P]...
  (--fresh-lines with info, export-png or layout-dump breaks every line anew
  instead of keeping the lines Hancom stored)
  heemang save <doc> -o <out> [--touch]
  heemang edit <doc> --script <edits.json> -o <out> [--font-path P]...";

enum Fail {
    Usage(String),
    Open(String),
    Render(String),
    Write(String),
}

impl Fail {
    fn code(&self) -> u8 {
        match self {
            Fail::Usage(_) => 1,
            Fail::Open(_) => 2,
            Fail::Render(_) => 3,
            Fail::Write(_) => 4,
        }
    }
}

#[derive(Default)]
struct Args {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    fonts: Vec<PathBuf>,
    dpi: f32,
    /// `--dpi` was given.
    dpi_set: bool,
    page: Option<usize>,
    audit: bool,
    touch: bool,
    timing: bool,
    fresh_lines: bool,
    script: Option<PathBuf>,
}

fn parse(argv: &[String]) -> Result<Args, Fail> {
    let mut a = Args {
        dpi: 96.0,
        ..Default::default()
    };
    let mut it = argv.iter();
    let value = |it: &mut std::slice::Iter<'_, String>, flag: &str| {
        it.next()
            .cloned()
            .ok_or_else(|| Fail::Usage(format!("{flag} needs a value")))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" | "--output" => a.output = Some(value(&mut it, arg)?.into()),
            "--font-path" => a.fonts.push(value(&mut it, arg)?.into()),
            "--dpi" => {
                a.dpi_set = true;
                a.dpi = value(&mut it, arg)?
                    .parse()
                    .map_err(|_| Fail::Usage("bad --dpi".into()))?
            }
            "-p" | "--page" => {
                a.page = Some(
                    value(&mut it, arg)?
                        .parse()
                        .map_err(|_| Fail::Usage("bad page".into()))?,
                )
            }
            "--audit" => a.audit = true,
            "--touch" => a.touch = true,
            "--script" => a.script = Some(value(&mut it, arg)?.into()),
            "--timing" => a.timing = true,
            "--fresh-lines" => a.fresh_lines = true,
            s if s.starts_with('-') => return Err(Fail::Usage(format!("unknown option {s}"))),
            s if a.input.is_none() => a.input = Some(s.into()),
            s => return Err(Fail::Usage(format!("unexpected argument {s}"))),
        }
    }
    Ok(a)
}

fn open(path: &Path) -> Result<Document, Fail> {
    let bytes = std::fs::read(path).map_err(|e| Fail::Open(format!("{}: {e}", path.display())))?;
    let fail = |e: &dyn std::fmt::Display| Fail::Open(format!("{}: {e}", path.display()));
    if hm_hwp5::is_hwp5(&bytes) {
        hm_hwp5::read(SharedBytes::from(bytes)).map_err(|e| fail(&e))
    } else if hm_hwpx::is_hwpx(&bytes) {
        hm_hwpx::read(SharedBytes::from(bytes)).map_err(|e| fail(&e))
    } else if hm_hwp3::is_hwp3(&bytes) {
        hm_hwp3::read(SharedBytes::from(bytes)).map_err(|e| fail(&e))
    } else {
        Err(Fail::Open(format!(
            "{}: not an HWP 5, HWPX or HWP 3 document",
            path.display()
        )))
    }
}

/// Registers fonts: files, or directories walked to any depth. A directory
/// of Hancom HFT fonts also brings Hancom's code conversion tables, which
/// live in `Bin/Code` next to the application's `Shared/Fonts`, and the font
/// mapping table `FontMap.dat` and HFT catalog `hftinfo.dat` beside the
/// fonts.
fn load_fonts(paths: &[PathBuf]) -> FontSystem {
    let mut fs = FontSystem::new();
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            add_dir(&mut fs, p, &mut files, 0);
        } else {
            files.push(p.clone());
        }
    }
    for f in files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let label = f.display().to_string();
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc" | "hft") {
            continue;
        }
        let Ok(file) = std::fs::File::open(&f) else {
            continue;
        };
        // SAFETY: font files are read-only inputs for the life of the process.
        let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
            continue;
        };
        fs.add(label, Arc::new(map));
    }
    fs
}

/// Collects the files under `dir` in name order, subdirectories after the
/// directory's own files, and registers the tables an HFT directory brings.
fn add_dir(fs: &mut FontSystem, dir: &Path, files: &mut Vec<PathBuf>, depth: u32) {
    // Deep enough for any font tree; stops symlink cycles.
    if depth > 16 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    let (dirs, here): (Vec<PathBuf>, Vec<PathBuf>) = entries.into_iter().partition(|p| p.is_dir());
    let has_hft = here
        .iter()
        .any(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("hft")));
    if has_hft {
        load_code_tables(fs, &dir.join("../../Bin/Code"));
        if let Ok(bytes) = std::fs::read(dir.join("FontMap.dat")) {
            fs.add_font_map(&bytes);
        }
        if let Ok(bytes) = std::fs::read(dir.join("hftinfo.dat")) {
            fs.add_hft_info(&bytes);
        }
    }
    files.extend(here.into_iter().filter(|p| p.is_file()));
    for d in dirs {
        add_dir(fs, &d, files, depth + 1);
    }
}

fn load_code_tables(fs: &mut FontSystem, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("u300_")
            && let Ok(bytes) = std::fs::read(&p)
        {
            fs.add_code_table(&bytes);
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(1);
    };
    let result = parse(&argv[1..]).and_then(|a| match cmd.as_str() {
        "info" => info(&a),
        "export-png" => export_png(&a),
        "export-svg" => export_svg(&a),
        "export-pdf" => export_pdf(&a),
        "export-text" => export_text(&a, false),
        "export-markdown" => export_text(&a, true),
        "layout-dump" => layout_dump(&a),
        "save" => save(&a),
        "edit" => edit(&a),
        _ => Err(Fail::Usage(format!("unknown command {cmd}"))),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            match &f {
                Fail::Usage(m) => eprintln!("heemang: {m}\n{USAGE}"),
                Fail::Open(m) | Fail::Render(m) | Fail::Write(m) => eprintln!("heemang: {m}"),
            }
            ExitCode::from(f.code())
        }
    }
}

fn input(a: &Args) -> Result<&Path, Fail> {
    a.input
        .as_deref()
        .ok_or_else(|| Fail::Usage("missing <doc>".into()))
}

fn info(a: &Args) -> Result<(), Fail> {
    let doc = open(input(a)?)?;
    let fonts = load_fonts(&a.fonts);
    let layout = hm_layout::layout_with(&doc, &fonts, options(a));
    let i = &doc.info;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "format: HWP {} (flags {:#x})",
        doc.meta.version, doc.meta.flags
    );
    let _ = writeln!(s, "sections: {}", doc.sections.len());
    for (k, sec) in doc.sections.iter().enumerate() {
        let paras = sec.paras.len();
        let (w, h) = sec
            .definition()
            .and_then(|d| d.page_def())
            .map_or((0, 0), |p| p.paper());
        let pages = layout
            .pages
            .iter()
            .filter(|p| p.section as usize == k)
            .count();
        let _ = writeln!(
            s,
            "  section {k}: {paras} paragraphs, paper {w}x{h} HU, {pages} pages"
        );
    }
    let faces: usize = i.faces.iter().map(Vec::len).sum();
    let _ = writeln!(
        s,
        "tables: {faces} faces, {} char shapes, {} para shapes, {} styles, {} border fills, {} tab defs, {} numberings, {} bullets, {} BinData",
        i.char_shapes.len(),
        i.para_shapes.len(),
        i.styles.len(),
        i.border_fills.len(),
        i.tab_defs.len(),
        i.numberings.len(),
        i.bullets.len(),
        i.bin_data.len()
    );
    let _ = writeln!(s, "pages: {}", layout.pages.len());
    if !layout.substitutions.is_empty() {
        let list: Vec<String> = layout
            .substitutions
            .iter()
            .map(|(a, b)| format!("{a} -> {b}"))
            .collect();
        let _ = writeln!(s, "substituted faces: {}", list.join(", "));
    }
    for d in doc.diagnostics.iter().take(20) {
        let _ = writeln!(s, "note: {}", d.message);
    }
    print!("{s}");
    Ok(())
}

fn options(a: &Args) -> hm_layout::Options {
    hm_layout::Options {
        stored_lines: !a.fresh_lines,
    }
}

fn layout_with_timing(a: &Args) -> Result<(Document, FontSystem, DocLayout), Fail> {
    let t0 = Instant::now();
    let fonts = load_fonts(&a.fonts);
    let t1 = Instant::now();
    let doc = open(input(a)?)?;
    let t2 = Instant::now();
    let layout = hm_layout::layout_with(&doc, &fonts, options(a));
    let t3 = Instant::now();
    if a.timing {
        eprintln!(
            "fonts {:.1} ms, open {:.1} ms, layout {:.1} ms, {} pages",
            (t1 - t0).as_secs_f64() * 1e3,
            (t2 - t1).as_secs_f64() * 1e3,
            (t3 - t2).as_secs_f64() * 1e3,
            layout.pages.len()
        );
    }
    Ok((doc, fonts, layout))
}

fn export_png(a: &Args) -> Result<(), Fail> {
    let out = a
        .output
        .clone()
        .ok_or_else(|| Fail::Usage("export-png needs -o <dir>".into()))?;
    let (doc, fonts, layout) = layout_with_timing(a)?;
    std::fs::create_dir_all(&out).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))?;
    let pages: Vec<usize> = match a.page {
        Some(p) if p < layout.pages.len() => vec![p],
        Some(p) => {
            return Err(Fail::Usage(format!(
                "page {p} out of range ({} pages)",
                layout.pages.len()
            )));
        }
        None => (0..layout.pages.len()).collect(),
    };
    let t = Instant::now();
    let threads = threads();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failure = std::sync::Mutex::new(None::<Fail>);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&p) = pages.get(k) else { break };
                    let dl = hm_render::build_page(&doc, &layout, &fonts, p);
                    let png = hm_render::raster::render_png(&dl, &fonts, a.dpi);
                    let result = match png {
                        Some(png) => {
                            let path = out.join(format!("page_{:03}.png", p + 1));
                            std::fs::write(&path, png)
                                .map_err(|e| Fail::Write(format!("{}: {e}", path.display())))
                        }
                        None => Err(Fail::Render(format!(
                            "page {} could not be rendered",
                            p + 1
                        ))),
                    };
                    if let Err(e) = result {
                        *failure.lock().unwrap() = Some(e);
                        break;
                    }
                }
            });
        }
    });
    if let Some(f) = failure.into_inner().unwrap() {
        return Err(f);
    }
    if a.timing {
        eprintln!(
            "render {:.1} ms for {} pages",
            t.elapsed().as_secs_f64() * 1e3,
            pages.len()
        );
    }
    Ok(())
}

/// Worker threads for page rendering.
fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
}

/// The pages `-p` selects, all by default.
fn selected(a: &Args, layout: &DocLayout) -> Result<Vec<usize>, Fail> {
    match a.page {
        Some(p) if p < layout.pages.len() => Ok(vec![p]),
        Some(p) => Err(Fail::Usage(format!(
            "page {p} out of range ({} pages)",
            layout.pages.len()
        ))),
        None => Ok((0..layout.pages.len()).collect()),
    }
}

/// The resolution vector exports render pictures at.
fn patch_dpi(a: &Args) -> f32 {
    if a.dpi_set { a.dpi } else { 200.0 }
}

fn export_svg(a: &Args) -> Result<(), Fail> {
    let out = a
        .output
        .clone()
        .ok_or_else(|| Fail::Usage("export-svg needs -o <dir>".into()))?;
    let (doc, fonts, layout) = layout_with_timing(a)?;
    std::fs::create_dir_all(&out).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))?;
    let pages = selected(a, &layout)?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failure = std::sync::Mutex::new(None::<Fail>);
    std::thread::scope(|scope| {
        for _ in 0..threads() {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&p) = pages.get(k) else { break };
                    let dl = hm_render::build_page(&doc, &layout, &fonts, p);
                    let path = out.join(format!("page_{:03}.svg", p + 1));
                    if let Err(e) =
                        std::fs::write(&path, hm_render::svg::page(&dl, &fonts, patch_dpi(a)))
                    {
                        *failure.lock().unwrap() =
                            Some(Fail::Write(format!("{}: {e}", path.display())));
                        break;
                    }
                }
            });
        }
    });
    failure.into_inner().unwrap().map_or(Ok(()), Err)
}

fn export_pdf(a: &Args) -> Result<(), Fail> {
    let out = a
        .output
        .clone()
        .ok_or_else(|| Fail::Usage("export-pdf needs -o <file.pdf>".into()))?;
    let (doc, fonts, layout) = layout_with_timing(a)?;
    let mut w = hm_render::pdf::Writer::new(&fonts, patch_dpi(a));
    for p in selected(a, &layout)? {
        w.page(&hm_render::build_page(&doc, &layout, &fonts, p));
    }
    std::fs::write(&out, w.finish()).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))
}

fn export_text(a: &Args, markdown: bool) -> Result<(), Fail> {
    let input = a
        .input
        .as_ref()
        .ok_or_else(|| Fail::Usage("missing <doc>".into()))?;
    let doc = open(input)?;
    let text = if markdown {
        hm_edit::export::markdown(&doc)
    } else {
        hm_edit::export::plain_text(&doc)
    };
    match &a.output {
        Some(out) => {
            std::fs::write(out, text).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))
        }
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

fn layout_dump(a: &Args) -> Result<(), Fail> {
    let (doc, _fonts, layout) = layout_with_timing(a)?;
    let mut s = String::new();
    s.push_str("{\"pages\":[");
    for (pi, page) in layout.pages.iter().enumerate() {
        if a.page.is_some_and(|p| p != pi) {
            continue;
        }
        if !s.ends_with('[') {
            s.push(',');
        }
        let _ = write!(
            s,
            "\n{{\"page\":{},\"section\":{},\"number\":{},\"width\":{},\"height\":{},\"body\":[{},{},{},{}],\"frames\":[",
            pi + 1,
            page.section,
            page.number,
            page.width,
            page.height,
            page.body.x,
            page.body.y,
            page.body.w,
            page.body.h
        );
        for (k, f) in page.frames.iter().enumerate() {
            let _ = write!(
                s,
                "{}{{\"kind\":\"{:?}\",\"list\":\"{:?}\",\"rect\":[{},{},{},{}],\"lines\":{},\"objects\":{}}}",
                if k > 0 { "," } else { "" },
                f.kind,
                f.list,
                f.rect.x,
                f.rect.y,
                f.rect.w,
                f.rect.h,
                f.lines.len(),
                f.objects.len()
            );
        }
        s.push_str("],\"lines\":[");
        for (k, pl) in page.lines.iter().enumerate() {
            let l = &layout.para(page.section, pl.para).lines[pl.line as usize];
            let para = &doc.sections[page.section as usize].paras[pl.para as usize];
            let text: String = para.text[l.start as usize..(l.end as usize).min(para.text.len())]
                .iter()
                .map(|&u| {
                    char::from_u32(u as u32)
                        .filter(|c| !c.is_control())
                        .unwrap_or(' ')
                })
                .collect();
            let stored = para
                .line_segs
                .get(pl.line as usize)
                .map_or("null".to_string(), |g| {
                    format!(
                        "[{},{},{},{},{},{},{},{},{}]",
                        g.text_start,
                        g.vpos,
                        g.height,
                        g.text_height,
                        g.baseline,
                        g.spacing,
                        g.col_start,
                        g.width,
                        g.flags
                    )
                });
            let _ = write!(
                s,
                "{}\n {{\"para\":{},\"line\":{},\"x\":{},\"y\":{},\"vpos\":{},\"start\":{},\"end\":{},\"lx\":{},\"width\":{},\"used\":{},\"height\":{},\"baseline\":{},\"spacing\":{},\"stored\":{},\"text\":{}}}",
                if k > 0 { "," } else { "" },
                pl.para,
                pl.line,
                pl.x,
                pl.y,
                pl.vpos,
                l.start,
                l.end,
                l.seg_x,
                l.seg_width,
                l.used,
                l.height,
                l.baseline,
                l.spacing,
                stored,
                json_str(&text)
            );
        }
        s.push_str("],\"objects\":[");
        for (k, o) in page.objects.iter().enumerate() {
            let _ = write!(
                s,
                "{}\n {{\"para\":{},\"control\":{},\"rect\":[{},{},{},{}],\"z\":{},\"layer\":\"{:?}\"}}",
                if k > 0 { "," } else { "" },
                o.para,
                o.control,
                o.rect.x,
                o.rect.y,
                o.rect.w,
                o.rect.h,
                o.z,
                o.layer
            );
        }
        s.push_str("],\"tables\":[");
        for (k, t) in page.tables.iter().enumerate() {
            let _ = write!(
                s,
                "{}\n {{\"para\":{},\"control\":{},\"x\":{},\"y\":{},\"top\":{},\"bottom\":{},\"height\":{}}}",
                if k > 0 { "," } else { "" },
                t.para,
                t.control,
                t.x,
                t.y,
                t.top,
                t.bottom,
                t.height()
            );
        }
        s.push_str("]}");
    }
    s.push_str("\n]");
    if a.audit {
        let au = hm_layout::audit(&doc, &layout, 20);
        s.push_str(",\n\"audit\":");
        audit_json(&mut s, &au);
    }
    s.push_str("}\n");
    print!("{s}");
    Ok(())
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn audit_json(s: &mut String, a: &Audit) {
    let r = |n| Audit::rate(n, a.lines);
    let _ = write!(
        s,
        "{{\"paras\":{},\"breaks_match_pct\":{:.2},\"exact_paras\":{},\"exact_breaks_match_pct\":{:.2},\"lines\":{},\"height_pct\":{:.2},\"text_height_pct\":{:.2},\"baseline_pct\":{:.2},\"spacing_pct\":{:.2},\"x_pct\":{:.2},\"width_pct\":{:.2},\"vpos_pct\":{:.2},\"diverging\":[",
        a.paras,
        Audit::rate(a.paras_breaks_match, a.paras),
        a.exact_paras,
        Audit::rate(a.exact_breaks_match, a.exact_paras),
        a.lines,
        r(a.height_match),
        r(a.text_height_match),
        r(a.baseline_match),
        r(a.spacing_match),
        r(a.x_match),
        r(a.width_match),
        r(a.vpos_match)
    );
    for (k, (sec, para, ours, stored)) in a.diverging.iter().enumerate() {
        let _ = write!(
            s,
            "{}\n {{\"section\":{sec},\"para\":{para},\"ours\":{ours:?},\"stored\":{stored:?}}}",
            if k > 0 { "," } else { "" }
        );
    }
    s.push_str("]}");
}

fn save(a: &Args) -> Result<(), Fail> {
    let out = a
        .output
        .clone()
        .ok_or_else(|| Fail::Usage("save needs -o <out>".into()))?;
    let mut doc = open(input(a)?)?;
    if a.touch {
        doc.mark_all_dirty();
    }
    let bytes = match doc.meta.format {
        hm_model::Format::Hwp5 => hm_hwp5::write(&doc).map_err(|e| Fail::Write(e.to_string()))?,
        hm_model::Format::Hwpx => hm_hwpx::write(&doc).map_err(|e| Fail::Write(e.to_string()))?,
        hm_model::Format::Hwp3 => {
            return Err(Fail::Write("HWP 3 documents cannot be saved yet".into()));
        }
    };
    std::fs::write(&out, bytes).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))?;
    Ok(())
}

/// Applies an edit script, lays the document out, gives edited paragraphs
/// our line segments and saves it in its own format.
fn edit(a: &Args) -> Result<(), Fail> {
    let out = a
        .output
        .clone()
        .ok_or_else(|| Fail::Usage("edit needs -o <out>".into()))?;
    let script_path = a
        .script
        .clone()
        .ok_or_else(|| Fail::Usage("edit needs --script <edits.json>".into()))?;
    let script = std::fs::read_to_string(&script_path)
        .map_err(|e| Fail::Open(format!("{}: {e}", script_path.display())))?;
    let doc = open(input(a)?)?;
    let mut ed = hm_edit::Editor::new(doc);
    hm_edit::script::run(&mut ed, &script).map_err(|e| Fail::Usage(e.to_string()))?;
    let fonts = load_fonts(&a.fonts);
    let layout = hm_layout::layout_with(&ed.doc, &fonts, options(a));
    hm_edit::lines::fill(&mut ed.doc, &layout);
    let doc = &ed.doc;
    let bytes = match doc.meta.format {
        hm_model::Format::Hwp5 => hm_hwp5::write(doc).map_err(|e| Fail::Write(e.to_string()))?,
        hm_model::Format::Hwpx => hm_hwpx::write(doc).map_err(|e| Fail::Write(e.to_string()))?,
        hm_model::Format::Hwp3 => {
            return Err(Fail::Write("HWP 3 documents cannot be saved yet".into()));
        }
    };
    std::fs::write(&out, bytes).map_err(|e| Fail::Write(format!("{}: {e}", out.display())))?;
    Ok(())
}
