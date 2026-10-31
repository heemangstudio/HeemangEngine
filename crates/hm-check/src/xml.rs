//! A strict XML 1.0 well-formedness parser building a small element tree:
//! UTF-8, legal characters, entity and character references, matching
//! tags, unique attributes and declared namespace prefixes.

pub struct El {
    /// Qualified name as written.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<usize>,
}

impl El {
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == k || n.rsplit(':').next() == Some(k))
            .map(|(_, v)| v.as_str())
    }
}

pub struct Doc {
    /// Elements in document order; 0 is the root.
    pub els: Vec<El>,
}

impl Doc {
    pub fn root(&self) -> &El {
        &self.els[0]
    }

    pub fn kids<'a>(&'a self, e: &'a El) -> impl Iterator<Item = &'a El> + 'a {
        e.kids.iter().map(|&k| &self.els[k])
    }
}

fn legal(c: char) -> bool {
    matches!(c as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':' || (c as u32) >= 0xC0
}

fn name_char(c: char) -> bool {
    name_start(c) || c.is_ascii_digit() || matches!(c, '-' | '.' | '\u{B7}')
}

struct P<'a> {
    s: &'a str,
    i: usize,
}

impl P<'_> {
    fn err<T>(&self, m: impl std::fmt::Display) -> Result<T, String> {
        let line = self.s[..self.i].matches('\n').count() + 1;
        Err(format!("{m} (line {line})"))
    }

    fn rest(&self) -> &str {
        &self.s[self.i..]
    }

    fn eat(&mut self, t: &str) -> bool {
        if self.rest().starts_with(t) {
            self.i += t.len();
            true
        } else {
            false
        }
    }

    fn ws(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if matches!(c, ' ' | '\t' | '\r' | '\n') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        let mut first = true;
        while let Some(c) = self.rest().chars().next() {
            if (first && name_start(c)) || (!first && name_char(c)) {
                self.i += c.len_utf8();
                first = false;
            } else {
                break;
            }
        }
        if start == self.i {
            return self.err("expected a name");
        }
        Ok(self.s[start..self.i].to_string())
    }

    /// Text up to `<` (or `quote`), with references resolved.
    fn chars(&mut self, quote: Option<char>) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let Some(c) = self.rest().chars().next() else {
                return match quote {
                    Some(_) => self.err("unterminated attribute value"),
                    None => Ok(out),
                };
            };
            if Some(c) == quote || (quote.is_none() && c == '<') {
                return Ok(out);
            }
            if c == '<' {
                return self.err("'<' in an attribute value");
            }
            if c == '&' {
                let end = match self.rest().find(';') {
                    Some(e) if e <= 12 => e,
                    _ => return self.err("'&' without a reference"),
                };
                let r = &self.rest()[1..end];
                let v = match r {
                    "lt" => '<',
                    "gt" => '>',
                    "amp" => '&',
                    "quot" => '"',
                    "apos" => '\'',
                    _ if r.starts_with("#x") => u32::from_str_radix(&r[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                        .filter(|&c| legal(c))
                        .map_or_else(|| self.err(format!("bad character reference &{r};")), Ok)?,
                    _ if r.starts_with('#') => r[1..]
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .filter(|&c| legal(c))
                        .map_or_else(|| self.err(format!("bad character reference &{r};")), Ok)?,
                    _ => return self.err(format!("undefined entity &{r};")),
                };
                out.push(v);
                self.i += end + 1;
                continue;
            }
            if !legal(c) {
                return self.err(format!("illegal character U+{:04X}", c as u32));
            }
            if quote.is_none() && self.rest().starts_with("]]>") {
                return self.err("']]>' in text");
            }
            out.push(c);
            self.i += c.len_utf8();
        }
    }

    fn until(&mut self, end: &str, what: &str) -> Result<(), String> {
        match self.rest().find(end) {
            Some(k) => {
                if let Some(c) = self.rest()[..k].chars().find(|&c| !legal(c)) {
                    return self.err(format!("illegal character U+{:04X} in {what}", c as u32));
                }
                self.i += k + end.len();
                Ok(())
            }
            None => self.err(format!("unterminated {what}")),
        }
    }

    /// Comments, processing instructions and whitespace between markup.
    fn misc(&mut self) -> Result<(), String> {
        loop {
            self.ws();
            if self.eat("<!--") {
                let k = self.rest().find("-->").map_or(usize::MAX, |k| k);
                if self.rest()[..k.min(self.rest().len())].contains("--") {
                    return self.err("'--' inside a comment");
                }
                self.until("-->", "comment")?;
            } else if self.rest().starts_with("<?") {
                self.i += 2;
                self.until("?>", "processing instruction")?;
            } else {
                return Ok(());
            }
        }
    }
}

pub fn parse(bytes: &[u8]) -> Result<Doc, String> {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let s = std::str::from_utf8(b).map_err(|e| format!("not UTF-8: {e}"))?;
    let mut p = P { s, i: 0 };
    if p.rest().starts_with("<?xml") {
        p.until("?>", "XML declaration")?;
    }
    p.misc()?;
    if p.rest().starts_with("<!DOCTYPE") {
        return p.err("document type declarations are not used in OWPML");
    }
    let mut els: Vec<El> = Vec::new();
    // Open elements with the namespace prefixes each declared.
    let mut stack: Vec<(usize, Vec<String>)> = Vec::new();
    let mut prefixes: Vec<String> = vec!["xml".into(), "xmlns".into()];
    loop {
        if stack.is_empty() && !els.is_empty() {
            p.misc()?;
            if !p.rest().is_empty() {
                return p.err("content after the root element");
            }
            break;
        }
        if stack.is_empty() {
            if !p.eat("<") {
                return p.err("no root element");
            }
        } else {
            p.chars(None)?;
            if p.rest().is_empty() {
                return p.err(format!(
                    "unclosed element <{}>",
                    els[stack.last().unwrap().0].name
                ));
            }
            p.i += 1; // '<'
            if p.eat("!--") {
                let k = p.rest().find("-->").unwrap_or(p.rest().len());
                if p.rest()[..k].contains("--") {
                    return p.err("'--' inside a comment");
                }
                p.until("-->", "comment")?;
                continue;
            }
            if p.eat("![CDATA[") {
                p.until("]]>", "CDATA section")?;
                continue;
            }
            if p.eat("?") {
                p.until("?>", "processing instruction")?;
                continue;
            }
            if p.eat("/") {
                let n = p.name()?;
                p.ws();
                if !p.eat(">") {
                    return p.err(format!("bad end tag </{n}"));
                }
                let (open, declared) = stack.pop().unwrap();
                if els[open].name != n {
                    return p.err(format!("</{n}> closes <{}>", els[open].name));
                }
                prefixes.truncate(prefixes.len() - declared.len());
                continue;
            }
        }
        let name = p.name()?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        let self_closed;
        loop {
            let had_ws = p.rest().starts_with([' ', '\t', '\r', '\n']);
            p.ws();
            if p.eat("/>") {
                self_closed = true;
                break;
            }
            if p.eat(">") {
                self_closed = false;
                break;
            }
            if !had_ws {
                return p.err(format!(
                    "attributes of <{name}> need whitespace between them"
                ));
            }
            let k = p.name()?;
            p.ws();
            if !p.eat("=") {
                return p.err(format!("attribute {k} of <{name}> has no value"));
            }
            p.ws();
            let q = match p.rest().chars().next() {
                Some(c @ ('"' | '\'')) => c,
                _ => return p.err(format!("attribute {k} of <{name}> is not quoted")),
            };
            p.i += 1;
            let v = p.chars(Some(q))?;
            p.i += 1;
            if attrs.iter().any(|(a, _)| *a == k) {
                return p.err(format!("<{name}> repeats attribute {k}"));
            }
            attrs.push((k, v));
        }
        let declared: Vec<String> = attrs
            .iter()
            .filter_map(|(k, _)| k.strip_prefix("xmlns:").map(str::to_string))
            .collect();
        prefixes.extend(declared.iter().cloned());
        for n in std::iter::once(&name).chain(attrs.iter().map(|(k, _)| k)) {
            if let Some((pre, _)) = n.split_once(':')
                && !prefixes.iter().any(|d| d == pre)
            {
                return p.err(format!("prefix {pre} of {n} is not declared"));
            }
        }
        let i = els.len();
        els.push(El {
            name,
            attrs,
            kids: Vec::new(),
        });
        if let Some(&(parent, _)) = stack.last() {
            els[parent].kids.push(i);
        }
        if self_closed {
            prefixes.truncate(prefixes.len() - declared.len());
        } else {
            stack.push((i, declared));
        }
    }
    Ok(Doc { els })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn rejects_what_hancom_cannot_read() {
        assert!(parse(b"<a><b/></a>").is_ok());
        assert!(parse(b"<a>\x01</a>").is_err());
        assert!(parse(b"<a x=\"1\" x=\"2\"/>").is_err());
        assert!(parse(b"<a><b></a>").is_err());
        assert!(parse(b"<p:a/>").is_err());
        assert!(parse(b"<a>&nbsp;</a>").is_err());
        assert!(parse(b"<a>&#1;</a>").is_err());
        assert!(parse("<a>\u{FFFF}</a>".as_bytes()).is_err());
        assert!(parse(b"<a xmlns:p=\"u\"><p:b c=\"&lt;\"/></a>").is_ok());
    }
}
