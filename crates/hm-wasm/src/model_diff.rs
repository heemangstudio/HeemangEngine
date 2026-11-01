//! What a save lost: typed differences between the model that was written
//! and the model read back, in the same format or another one.
//!
//! The comparison leaves out what a writer recomputes or only one format's
//! bytes carry, and nothing else: source spans and undecoded record tails
//! (a picture's effects and additional properties are compared by meaning),
//! records and markup the model does not interpret, the record order and
//! the ID_MAPPINGS counts, paragraph counts of lists, the order of a
//! drawing's geometry and text records, the HWP 5 marks in the high bits of
//! a drawing's flip word and the caption bit of an object's attributes,
//! which writers derive from the caption, and the section and column
//! definition marks of a paragraph's break kind, which the HWP 5 writer
//! derives from its controls. An absent optional field equals the zero
//! value a writer fills in for it.

use hm_model::Document;
use serde_json::{Value, json};

/// Record and markup leftovers of the format read from.
const FORMAT_ONLY: &[&str] = &["tail", "extra", "raw", "xml", "data", "src"];

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            // A picture's tail holds its effects, alpha and original size.
            if let Some(pic) = m.get_mut("Picture")
                && let Ok(p) = serde_json::from_value::<hm_model::Picture>(pic.clone())
            {
                pic["meaning"] = json!({"effects": p.effect_list(), "alpha": p.alpha(),
                    "original": p.original_size().unwrap_or((0, 0))});
            }
            for k in FORMAT_ONLY {
                m.remove(*k);
            }
            // A list's paragraph count follows its paragraphs.
            if m.contains_key("paras") && m.contains_key("text_size") {
                m.remove("count");
            }
            if let Some(Value::Object(c)) = m.get_mut("component")
                && let Some(f) = c.get("flip").and_then(Value::as_u64)
            {
                c.insert("flip".into(), json!(f & 0xF_FFFF));
            }
            // A drawing's geometry and text list are records in the order its
            // writer puts them; its members keep their drawing order.
            if m.contains_key("component")
                && let Some(Value::Array(parts)) = m.get_mut("parts")
            {
                let kind = |p: &Value| {
                    p.as_object()
                        .and_then(|o| o.keys().next().cloned())
                        .unwrap_or_default()
                };
                let (mut own, members): (Vec<Value>, Vec<Value>) =
                    parts.drain(..).partition(|p| kind(p) != "Child");
                own.sort_by_key(kind);
                parts.extend(own);
                parts.extend(members);
            }
            if let Some(k) = m.get("break_kind").and_then(Value::as_u64) {
                m.insert("break_kind".into(), json!(k & !3));
            }
            for kind in ["Object", "Table"] {
                if let Some(Value::Object(o)) = m.get_mut(kind)
                    && let Some(Value::Object(h)) = o.get_mut("header")
                    && let Some(Value::Object(c)) = h.get_mut("common")
                    && let Some(a) = c.get("attr").and_then(Value::as_u64)
                {
                    c.insert("attr".into(), json!(a & !(1 << 29)));
                }
            }
            for v in m.values_mut() {
                strip(v);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        _ => {}
    }
}

/// A value nothing is set in: what a writer fills in for an absent field.
fn zero(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.iter().all(zero),
        Value::Object(m) => m.values().all(zero),
        Value::Bool(true) => false,
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 80 {
        let t: String = s.chars().take(80).collect();
        format!("{t}…")
    } else {
        s
    }
}

fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<(String, String)>) {
    if a == b || out.len() >= 5000 {
        return;
    }
    match (a, b) {
        (Value::Null, v) | (v, Value::Null) if zero(v) => {}
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                diff(
                    x.get(k).unwrap_or(&Value::Null),
                    y.get(k).unwrap_or(&Value::Null),
                    &format!("{path}/{k}"),
                    out,
                );
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(p, q, &format!("{path}/{i}"), out);
            }
        }
        _ => out.push((path.to_string(), format!("{} | {}", short(a), short(b)))),
    }
}

fn value<T: serde::Serialize>(t: &T) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    strip(&mut v);
    v
}

/// Every typed difference of `actual` from `expected`, as `(path, values)`:
/// `info/...` for the document tables, `s<k>/p<n>/...` for section `k`'s
/// paragraph `n` (and the lists inside it), `bins/<id>` for embedded data.
#[doc(hidden)]
pub fn model_differences(expected: &Document, actual: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut a, mut b) = (value(&expected.info), value(&actual.info));
    for v in [&mut a, &mut b] {
        if let Value::Object(m) = v {
            m.remove("order");
            m.remove("id_mappings");
            m.remove("imported_line_membership");
        }
    }
    diff(&a, &b, "info", &mut out);
    if expected.sections.len() != actual.sections.len() {
        out.push((
            "sections".into(),
            format!("{} | {}", expected.sections.len(), actual.sections.len()),
        ));
    }
    for (si, (e, x)) in expected.sections.iter().zip(&actual.sections).enumerate() {
        if e.paras.len() != x.paras.len() {
            out.push((
                format!("s{si}/paras"),
                format!("{} | {}", e.paras.len(), x.paras.len()),
            ));
        }
        for (pi, (ep, xp)) in e.paras.iter().zip(&x.paras).enumerate() {
            diff(&value(ep), &value(xp), &format!("s{si}/p{pi}"), &mut out);
        }
        diff(
            &value(&e.last_page_master),
            &value(&x.last_page_master),
            &format!("s{si}/last_page_master"),
            &mut out,
        );
        diff(
            &value(&e.page_masters),
            &value(&x.page_masters),
            &format!("s{si}/page_masters"),
            &mut out,
        );
    }
    let n = expected.info.bin_data.len().max(actual.info.bin_data.len());
    for id in 1..=n as u16 {
        let get = |d: &Document| d.bins.as_ref().and_then(|b| b.get(id));
        if get(expected).as_deref() != get(actual).as_deref() {
            out.push((format!("bins/{id}"), "bytes differ".into()));
        }
    }
    out
}

/// The paths of [`model_differences`].
#[doc(hidden)]
pub fn model_losses(expected: &Document, actual: &Document) -> Vec<String> {
    model_differences(expected, actual)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}
