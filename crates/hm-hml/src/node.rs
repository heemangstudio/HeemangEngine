//! XML values shared by the HWPML dialect mappings.
use hm_hwpx::xml::{El, Kind, escape_attr, escape_text};

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}
impl Node {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
    pub fn a(mut self, key: impl Into<String>, value: impl std::fmt::Display) -> Self {
        self.set(key, value);
        self
    }
    pub fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    pub fn set(&mut self, key: impl Into<String>, value: impl std::fmt::Display) {
        let key = key.into();
        let value = value.to_string();
        if let Some((_, v)) = self.attrs.iter_mut().find(|(k, _)| k == &key) {
            *v = value;
        } else {
            self.attrs.push((key, value));
        }
    }
    pub fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|n| n.name == name)
    }
    pub fn from_el(el: El<'_>) -> Self {
        if !el.is_element() {
            return Self {
                name: "#text".into(),
                text: if el.kind() == Kind::CData {
                    String::from_utf8_lossy(el.content()).into_owned()
                } else {
                    hm_hwpx::xml::unescape(el.content()).into_owned()
                },
                ..Self::default()
            };
        }
        let mut out = Self::new(String::from_utf8_lossy(el.local()).into_owned());
        out.attrs = el
            .attrs()
            .map(|(_, k, v)| {
                (
                    String::from_utf8_lossy(k).into_owned(),
                    hm_hwpx::xml::unescape(v).into_owned(),
                )
            })
            .collect();
        if el.children().next().is_none() {
            out.text = el.text();
        } else {
            out.children = el.nodes().map(Self::from_el).collect();
        }
        out
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }
    pub fn write(&self, out: &mut Vec<u8>) {
        if self.name == "#text" {
            escape_text(out, &self.text);
            return;
        }
        out.push(b'<');
        out.extend(self.name.as_bytes());
        for (k, v) in &self.attrs {
            out.push(b' ');
            out.extend(k.as_bytes());
            out.extend(b"=\"");
            escape_attr(out, v);
            out.push(b'"');
        }
        if self.children.is_empty() && self.text.is_empty() {
            out.extend(b"/>");
            return;
        }
        out.push(b'>');
        escape_text(out, &self.text);
        for c in &self.children {
            c.write(out);
        }
        out.extend(b"</");
        out.extend(self.name.as_bytes());
        out.push(b'>');
    }
}
pub fn color_up(value: &str) -> String {
    if value.starts_with('#') || value.eq_ignore_ascii_case("none") {
        return value.into();
    }
    let Ok(c) = value.parse::<u32>() else {
        return value.into();
    };
    if c == u32::MAX {
        return "none".into();
    }
    let alpha = if c >> 24 == 0 {
        String::new()
    } else {
        format!("{:02X}", c >> 24)
    };
    format!(
        "#{alpha}{:02X}{:02X}{:02X}",
        c & 255,
        (c >> 8) & 255,
        (c >> 16) & 255
    )
}
pub fn color_down(value: &str) -> String {
    if value == "none" {
        return u32::MAX.to_string();
    }
    if let Some(hex) = value.strip_prefix('#') {
        if let Ok(c) = u32::from_str_radix(hex, 16) {
            return (((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255) | (c & 0xff000000))
                .to_string();
        }
    }
    value.into()
}
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}
pub fn mapped(n: &Node, name: &str, attrs: &[(&str, &str)], up: bool) -> Node {
    let mut out = Node::new(name);
    out.text = n.text.clone();
    for (key, value) in &n.attrs {
        if key.starts_with("xmlns") {
            continue;
        }
        let key = attrs
            .iter()
            .find_map(|(a, b)| {
                if up && key == a {
                    Some((*b).to_string())
                } else if !up && key == b {
                    Some((*a).to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                let mut chars = key.chars();
                match chars.next() {
                    Some(c) => format!(
                        "{}{}",
                        if up {
                            c.to_ascii_lowercase()
                        } else {
                            c.to_ascii_uppercase()
                        },
                        chars.as_str()
                    ),
                    None => String::new(),
                }
            });
        let value = if key.to_ascii_lowercase().contains("color")
            || (local(&n.name).eq_ignore_ascii_case("color") && key.eq_ignore_ascii_case("value"))
        {
            if up {
                color_up(value)
            } else {
                color_down(value)
            }
        } else {
            value.clone()
        };
        out.attrs.push((key, value));
    }
    out
}
