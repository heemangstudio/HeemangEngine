//! Annotations: fields, bookmarks, index marks, form objects, memos,
//! overlaid characters (글자 겹치기) and ruby text (덧말).

use crate::{CtrlId, HwpString, ObjectHeader, ParaList, RawRecord, SharedBytes};

/// What a field computes or marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    ClickHere,
    Hyperlink,
    Bookmark,
    Formula,
    Summary,
    UserInfo,
    Date,
    DocDate,
    Path,
    CrossRef,
    MailMerge,
    Memo,
    PrivateInfo,
    TableOfContents,
    /// Proofreading marks (`%%*?`).
    Revision,
    Unknown,
}

impl FieldKind {
    pub fn of(id: CtrlId) -> FieldKind {
        match &id.chars() {
            b"%clk" => FieldKind::ClickHere,
            b"%hlk" => FieldKind::Hyperlink,
            b"%bmk" => FieldKind::Bookmark,
            b"%fmu" => FieldKind::Formula,
            b"%smr" => FieldKind::Summary,
            b"%usr" => FieldKind::UserInfo,
            b"%dte" => FieldKind::Date,
            b"%ddt" => FieldKind::DocDate,
            b"%pat" => FieldKind::Path,
            b"%xrf" => FieldKind::CrossRef,
            b"%mmg" => FieldKind::MailMerge,
            b"%cpr" => FieldKind::PrivateInfo,
            b"%toc" => FieldKind::TableOfContents,
            [b'%', b'%', b'*', _] => FieldKind::Revision,
            _ => FieldKind::Unknown,
        }
    }
}

/// A field start (control code 3). The field's text runs to the matching
/// field end (inline code 4).
#[derive(Clone, Debug, Default)]
pub struct Field {
    /// `%clk`, `%hlk`, ...
    pub id: CtrlId,
    /// Bit 0 editable in read-only mode, bits 11-14 hyperlink update kind,
    /// bit 15 the content changed.
    pub attr: u32,
    /// The extra attribute byte (Hancom's `Prop` parameter).
    pub props: u8,
    /// The kind's command string (click-here guide text, link target,
    /// formula, ...).
    pub command: HwpString,
    /// Document-unique field id.
    pub instance: u32,
    pub tail: SharedBytes,
    /// The field name.
    pub name: Option<HwpString>,
    /// HWP 5 control data records, kept as stored.
    pub data: Vec<RawRecord>,
    /// A memo field's text.
    pub memo: Option<ParaList>,
}

impl Field {
    pub fn kind(&self) -> FieldKind {
        match FieldKind::of(self.id) {
            FieldKind::Unknown if self.memo_number().is_some() => FieldKind::Memo,
            k => k,
        }
    }

    /// A memo's number: memo fields are unknown fields whose command is
    /// `MEMO/<shape>/<number>/...`; HWP 5 keys the memo text by it.
    pub fn memo_number(&self) -> Option<u32> {
        let command = self.command.to_string_lossy();
        let mut parts = command.strip_prefix("MEMO/")?.split('/');
        parts.next()?;
        parts.next()?.parse().ok()
    }

    pub fn editable(&self) -> bool {
        self.attr & 1 != 0
    }

    pub fn dirty(&self) -> bool {
        self.attr & (1 << 15) != 0
    }

    /// A click-here field's guide text: the `Direction` parameter of its
    /// command, or in the older form the command up to its `;`.
    pub fn guide(&self) -> Option<String> {
        let command = self.command.to_string_lossy();
        let params = Params::parse(&command);
        if params.items.is_empty() {
            return command
                .split(';')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        params
            .find_deep("Direction")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

/// A point bookmark (`bokm`).
#[derive(Clone, Debug, Default)]
pub struct Bookmark {
    pub name: Option<HwpString>,
    /// HWP 5 control data records, kept as stored.
    pub data: Vec<RawRecord>,
    pub tail: SharedBytes,
}

/// An index mark (`idxm`) with its two keywords.
#[derive(Clone, Debug, Default)]
pub struct IndexMark {
    pub keys: [HwpString; 2],
    pub tail: SharedBytes,
}

/// What a form object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    PushButton,
    CheckBox,
    RadioButton,
    ComboBox,
    ListBox,
    Edit,
    ScrollBar,
    Unknown,
}

/// A form object (`form`): an object header and Hancom's property sets.
#[derive(Clone, Debug, Default)]
pub struct FormObject {
    pub header: ObjectHeader,
    /// The form record's type tag (`tbp+`, `tbc+`, `tbr+`, `boc+`, `bol+`,
    /// `tde+`, `tbs+` as stored).
    pub kind: CtrlId,
    /// `CommonSet`, `CharShapeSet` and the kind's set (`ButtonSet`,
    /// `ComboBoxSet`, `EditSet`, ...).
    pub props: Params,
    /// Combo and list box entries: display text and value.
    pub items: Vec<(HwpString, HwpString)>,
    pub tail: SharedBytes,
}

impl FormObject {
    pub const PUSH_BUTTON: CtrlId = CtrlId::from_chars(b"+pbt");
    pub const CHECK_BOX: CtrlId = CtrlId::from_chars(b"+cbt");
    pub const RADIO_BUTTON: CtrlId = CtrlId::from_chars(b"+rbt");
    pub const COMBO_BOX: CtrlId = CtrlId::from_chars(b"+cob");
    pub const LIST_BOX: CtrlId = CtrlId::from_chars(b"+lob");
    pub const EDIT: CtrlId = CtrlId::from_chars(b"+edt");
    pub const SCROLL_BAR: CtrlId = CtrlId::from_chars(b"+sbt");

    pub fn form_kind(&self) -> FormKind {
        match self.kind {
            Self::PUSH_BUTTON => FormKind::PushButton,
            Self::CHECK_BOX => FormKind::CheckBox,
            Self::RADIO_BUTTON => FormKind::RadioButton,
            Self::COMBO_BOX => FormKind::ComboBox,
            Self::LIST_BOX => FormKind::ListBox,
            Self::EDIT => FormKind::Edit,
            Self::SCROLL_BAR => FormKind::ScrollBar,
            _ => FormKind::Unknown,
        }
    }

    /// A property by set and item name.
    pub fn prop(&self, set: &str, item: &str) -> Option<&ParamValue> {
        self.props.find_set(set)?.find(item)
    }

    fn common_int(&self, item: &str, default: i64) -> i64 {
        self.prop("CommonSet", item)
            .and_then(|v| v.as_int())
            .unwrap_or(default)
    }

    /// COLORREF of the face.
    pub fn back_color(&self) -> u32 {
        self.common_int("BackColor", 0xFFFFFF) as u32
    }

    /// COLORREF of marks and text.
    pub fn fore_color(&self) -> u32 {
        self.common_int("ForeColor", 0) as u32
    }

    /// 0 none, 2 bump, 3 etched, 4 raised, 5 sunken.
    pub fn border_type(&self) -> i64 {
        self.common_int("BorderType", 0)
    }

    pub fn printable(&self) -> bool {
        self.common_int("Printable", 1) != 0
    }

    pub fn char_shape(&self) -> u32 {
        self.prop("CharShapeSet", "CharShapeID")
            .and_then(|v| v.as_int())
            .unwrap_or(0) as u32
    }

    /// Check box and radio button state: 0 off, 1 on, 2 indeterminate.
    pub fn value(&self) -> i64 {
        self.prop("ButtonSet", "Value")
            .and_then(|v| v.as_int())
            .unwrap_or(0)
    }

    /// Check boxes and radio buttons paint their face unless transparent.
    pub fn opaque(&self) -> bool {
        self.prop("ButtonSet", "BackStyle")
            .and_then(|v| v.as_int())
            .unwrap_or(1)
            != 0
    }

    /// The text the control shows: a button's caption without its
    /// mnemonic marks, an edit box's or combo box's text (masked when the
    /// edit box has a password character).
    pub fn display_text(&self) -> String {
        let text = |set: &str| {
            self.prop(set, "Text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        match self.form_kind() {
            FormKind::Edit => {
                let t = text("EditSet");
                match self
                    .prop("EditSet", "PasswordChar")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.chars().next())
                {
                    Some(mask) => t.chars().map(|_| mask).collect(),
                    None => t,
                }
            }
            FormKind::ComboBox | FormKind::ListBox => {
                let t = text("ComboBoxSet");
                if t.is_empty() { text("ListBoxSet") } else { t }
            }
            _ => {
                let caption = self
                    .prop("ButtonSet", "Caption")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let mut out = String::new();
                let mut chars = caption.chars();
                while let Some(c) = chars.next() {
                    if c != '&' {
                        out.push(c);
                    } else if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                out
            }
        }
    }
}

/// Overlaid characters (글자 겹치기, `tcps`): characters set over each other
/// or inside a frame shape, taking one character's place in the line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compose {
    /// The characters, as stored.
    pub text: HwpString,
    /// Frame and inner glyph settings. Some writers store only the text;
    /// the characters are then set over each other without a frame.
    pub frame: Option<ComposeFrame>,
    pub tail: SharedBytes,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposeFrame {
    /// 0 none (the characters themselves overlap), 1 circle, 2 reversed
    /// circle, 3 rectangle, 4 reversed rectangle, 5 triangle, 6 reversed
    /// triangle, 7 light, 8 rhombus, 9 reversed rhombus, 10 rounded
    /// rectangle, 11 empty circulating triangle, 12 thin circulating triangle.
    pub shape: u8,
    /// Inner character size step, from -5 (smallest) to 2.
    pub inner_size: i8,
    /// 0 spread the characters inside the frame, 1 overlap them.
    pub kind: u8,
    /// Char shape per character (`u32::MAX`: the run's).
    pub char_shapes: Vec<u32>,
}

impl Compose {
    pub fn chars(&self) -> Vec<char> {
        char::decode_utf16(self.text.0.iter().copied())
            .filter_map(Result::ok)
            .collect()
    }

    /// The char shape of character `i`, when one is set for it.
    pub fn char_shape(&self, i: usize) -> Option<u32> {
        self.frame
            .as_ref()?
            .char_shapes
            .get(i)
            .copied()
            .filter(|&c| c != u32::MAX)
    }
}

/// Ruby text (덧말, `tdut`): a main text with an annotation above, below or
/// centered on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dutmal {
    pub main: HwpString,
    pub sub: HwpString,
    /// 0 above, 1 below, 2 center.
    pub position: u32,
    /// Annotation size in percent of the main text; 0 means 50.
    pub size_ratio: u32,
    pub option: u32,
    /// Style of the annotation when the option asks for one.
    pub style: u32,
    /// Annotation alignment over the main text, as paragraph alignment
    /// codes: 0 justify, 1 left, 2 right, 3 center, 4 distribute, 5 divide.
    pub align: u32,
    pub tail: SharedBytes,
}

impl Dutmal {
    /// The annotation's size in percent of the main text.
    pub fn ratio(&self) -> i32 {
        match self.size_ratio {
            0 => 50,
            r => r.min(1000) as i32,
        }
    }
}

/// A Hancom parameter set: named items holding numbers, flags, strings or
/// nested sets. HWP 5 stores form properties and click-here commands in
/// its text form, `Name:type:value` items separated by spaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params {
    pub items: Vec<(String, ParamValue)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamValue {
    Int(i64),
    Bool(bool),
    Str(String),
    Set(Params),
}

impl ParamValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ParamValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            ParamValue::Int(v) => Some(*v),
            ParamValue::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            ParamValue::Int(v) => Some(*v != 0),
            _ => None,
        }
    }
}

impl Params {
    pub fn find(&self, name: &str) -> Option<&ParamValue> {
        self.items.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// An item here or in any nested set.
    pub fn find_deep(&self, name: &str) -> Option<&ParamValue> {
        self.find(name).or_else(|| {
            self.items.iter().find_map(|(_, v)| match v {
                ParamValue::Set(p) => p.find_deep(name),
                _ => None,
            })
        })
    }

    pub fn find_set(&self, name: &str) -> Option<&Params> {
        match self.find(name)? {
            ParamValue::Set(p) => Some(p),
            _ => None,
        }
    }

    pub fn set(&mut self, name: &str, value: ParamValue) {
        match self.items.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value,
            None => self.items.push((name.to_string(), value)),
        }
    }

    /// Parses the text form. `wstring` and `set` carry a character count
    /// before their content; other types run to the next space.
    pub fn parse(text: &str) -> Params {
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        Self::parse_range(&chars, &mut i, chars.len())
    }

    fn parse_range(c: &[char], i: &mut usize, end: usize) -> Params {
        let mut p = Params::default();
        let token = |i: &mut usize| -> String {
            let s = *i;
            while *i < end && c[*i] != ':' && c[*i] != ' ' {
                *i += 1;
            }
            let t: String = c[s..*i].iter().collect();
            if *i < end && c[*i] == ':' {
                *i += 1;
            }
            t
        };
        while *i < end {
            while *i < end && c[*i] == ' ' {
                *i += 1;
            }
            if *i >= end {
                break;
            }
            let name = token(i);
            let kind = token(i);
            let value = match kind.as_str() {
                "wstring" | "set" => {
                    let n: usize = token(i).parse().unwrap_or(0);
                    let stop = (*i + n).min(end);
                    let v = if kind == "set" {
                        let mut j = *i;
                        ParamValue::Set(Self::parse_range(c, &mut j, stop))
                    } else {
                        ParamValue::Str(c[*i..stop].iter().collect())
                    };
                    *i = stop;
                    v
                }
                "bool" => ParamValue::Bool(token(i) != "0"),
                _ => {
                    let t = token(i);
                    match t.parse::<i64>() {
                        Ok(v) => ParamValue::Int(v),
                        Err(_) => ParamValue::Str(t),
                    }
                }
            };
            if name.is_empty() {
                break;
            }
            p.items.push((name, value));
        }
        p
    }

    /// The text form, as Hancom writes it: each item followed by a space.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (name, v) in &self.items {
            match v {
                ParamValue::Int(n) => s.push_str(&format!("{name}:int:{n} ")),
                ParamValue::Bool(b) => s.push_str(&format!("{name}:bool:{} ", *b as u8)),
                ParamValue::Str(t) => {
                    s.push_str(&format!("{name}:wstring:{}:{t} ", t.chars().count()))
                }
                ParamValue::Set(p) => {
                    let inner = p.to_text();
                    s.push_str(&format!("{name}:set:{}:{inner} ", inner.chars().count()));
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_text_round_trips() {
        let text = "CommonSet:set:218:Name:wstring:10:PushButton ForeColor:int:0 \
                    BackColor:int:15790320 GroupName:wstring:0: TabStop:bool:1 TabOrder:int:1 \
                    Enabled:bool:1 BorderType:int:4 DrawFrame:bool:1 Command:wstring:0: \
                    Editable:bool:1 Printable:bool:1  ButtonSet:set:24:Caption:wstring:5:명령 단추  ";
        let p = Params::parse(text);
        let common = p.find_set("CommonSet").unwrap();
        assert_eq!(
            common.find("Name").and_then(|v| v.as_str()),
            Some("PushButton")
        );
        assert_eq!(common.find("TabStop").and_then(|v| v.as_bool()), Some(true));
        let button = p.find_set("ButtonSet").unwrap();
        assert_eq!(
            button.find("Caption").and_then(|v| v.as_str()),
            Some("명령 단추")
        );
        assert_eq!(p.to_text(), text);
    }

    #[test]
    fn captions_drop_mnemonic_marks() {
        let form = |caption: &str| FormObject {
            kind: FormObject::CHECK_BOX,
            props: Params::parse(&format!(
                "ButtonSet:set:{}:Caption:wstring:{}:{caption} ",
                caption.chars().count() + 22,
                caption.chars().count()
            )),
            ..Default::default()
        };
        assert_eq!(form("IP R&&D연계").display_text(), "IP R&D연계");
        assert_eq!(form("&Save").display_text(), "Save");
    }

    #[test]
    fn click_here_guide() {
        let f = Field {
            id: CtrlId::from_chars(b"%clk"),
            command: HwpString::from_text(
                "Clickhere:set:42:Direction:wstring:8:여기 입력하세요 HelpState:wstring:0: ",
            ),
            ..Default::default()
        };
        assert_eq!(f.kind(), FieldKind::ClickHere);
        assert_eq!(f.guide().as_deref(), Some("여기 입력하세요"));
    }
}
