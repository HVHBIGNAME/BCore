//! Network NBT ("anonymous NBT") encoding for chat text components.
//!
//! Since 1.20.3 chat content travels as NBT rather than JSON. The protocol type
//! is `anonymousNbt`: a root tag id followed by the payload, with **no root
//! name** (unlike file NBT). Both shapes vanilla 26.2 uses are implemented here
//! and were verified against live captures (`scripts/capture_chat.py`):
//!
//! ```text
//! plain text  : 08 <u16 len> <utf8>                       ("/help [<command>]")
//! styled text : 0a 08 <u16 5> "color" <u16 3> "red" ... 00 ({color:red, text:""})
//! ```
//!
//! A bare `TAG_String` root is a complete component (vanilla sends command help
//! lines exactly that way), so [`encode_component`] emits the compact form
//! whenever no styling is attached and the compound form otherwise.
//!
//! NBT strings are length-prefixed with a big-endian `u16` byte count. Java
//! writes "modified UTF-8"; for text without NUL or surrogate pairs (everything
//! BCore produces) that is byte-identical to plain UTF-8.

/// NBT tag id: end of compound.
pub const TAG_END: u8 = 0;
/// NBT tag id: signed byte (used for boolean style flags).
pub const TAG_BYTE: u8 = 1;
/// NBT tag id: length-prefixed UTF-8 string.
pub const TAG_STRING: u8 = 8;
/// NBT tag id: list of same-typed payloads.
pub const TAG_LIST: u8 = 9;
/// NBT tag id: named-entry compound, terminated by [`TAG_END`].
pub const TAG_COMPOUND: u8 = 10;

/// Write an NBT string payload: big-endian `u16` byte length + bytes.
pub fn write_nbt_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Write a named compound entry header: tag id + entry name.
fn write_entry(tag: u8, name: &str, out: &mut Vec<u8>) {
    out.push(tag);
    write_nbt_string(name, out);
}

/// A minimal chat text component: literal text plus optional colour, italics
/// and child components.
///
/// This covers everything BCore needs to say. Richer vanilla features
/// (`translate`, `click_event`, `hover_event`) are deliberately absent: BCore
/// sends literal text so output does not depend on the client's language files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    pub text: String,
    pub color: Option<String>,
    pub italic: Option<bool>,
    pub extra: Vec<Component>,
}

impl Component {
    /// An unstyled literal-text component.
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            ..Self::default()
        }
    }

    /// A literal-text component in one of the vanilla named colours.
    pub fn colored(s: impl Into<String>, color: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            color: Some(color.into()),
            ..Self::default()
        }
    }

    /// Set the italic style flag.
    pub fn italic(mut self, italic: bool) -> Self {
        self.italic = Some(italic);
        self
    }

    /// Append a child component (rendered after this one's text).
    pub fn push(mut self, child: Component) -> Self {
        self.extra.push(child);
        self
    }

    /// True when no styling is attached, so the compact string form suffices.
    pub fn is_plain(&self) -> bool {
        self.color.is_none() && self.italic.is_none() && self.extra.is_empty()
    }
}

/// Encode a component as `anonymousNbt` (root tag id + payload, no root name).
///
/// Unstyled components become a bare `TAG_String` root — the exact shape
/// vanilla uses for command help output.
pub fn encode_component(component: &Component) -> Vec<u8> {
    let mut out = Vec::new();
    if component.is_plain() {
        out.push(TAG_STRING);
        write_nbt_string(&component.text, &mut out);
    } else {
        out.push(TAG_COMPOUND);
        write_compound_body(component, &mut out);
    }
    out
}

/// Encode a plain string component: `08 <u16 len> <utf8>`.
pub fn encode_text(text: &str) -> Vec<u8> {
    encode_component(&Component::text(text))
}

/// Write a component's compound entries plus the terminating [`TAG_END`].
///
/// Entry order is fixed (`color`, `italic`, `extra`, `text`) so the encoding is
/// byte-for-byte deterministic; NBT compounds are unordered, so the client is
/// indifferent to the choice.
fn write_compound_body(component: &Component, out: &mut Vec<u8>) {
    if let Some(color) = &component.color {
        write_entry(TAG_STRING, "color", out);
        write_nbt_string(color, out);
    }
    if let Some(italic) = component.italic {
        write_entry(TAG_BYTE, "italic", out);
        out.push(u8::from(italic));
    }
    if !component.extra.is_empty() {
        write_entry(TAG_LIST, "extra", out);
        // A TAG_List is homogeneous, so every child is written as a compound
        // even when it carries no styling of its own.
        out.push(TAG_COMPOUND);
        out.extend_from_slice(&(component.extra.len() as i32).to_be_bytes());
        for child in &component.extra {
            write_compound_body(child, out);
        }
    }
    // `text` is always present: vanilla emits it even when empty.
    write_entry(TAG_STRING, "text", out);
    write_nbt_string(&component.text, out);
    out.push(TAG_END);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_matches_the_vanilla_help_line_bytes() {
        // Captured from vanilla 26.2 replying to `/help`: system_chat content
        // was a bare TAG_String root holding "/me <action>".
        let want = hex("08000c2f6d65203c616374696f6e3e");
        assert_eq!(encode_text("/me <action>"), want);
    }

    #[test]
    fn plain_text_has_no_root_name() {
        let bytes = encode_text("hi");
        assert_eq!(bytes[0], TAG_STRING, "root tag id");
        assert_eq!(&bytes[1..3], &[0x00, 0x02], "length prefix follows the tag");
        assert_eq!(&bytes[3..], b"hi");
        assert_eq!(bytes.len(), 5);
    }

    #[test]
    fn colored_text_is_a_compound_with_color_and_text() {
        let bytes = encode_component(&Component::colored("boom", "red"));
        let mut want = vec![TAG_COMPOUND];
        want.push(TAG_STRING);
        want.extend_from_slice(&[0x00, 0x05]);
        want.extend_from_slice(b"color");
        want.extend_from_slice(&[0x00, 0x03]);
        want.extend_from_slice(b"red");
        want.push(TAG_STRING);
        want.extend_from_slice(&[0x00, 0x04]);
        want.extend_from_slice(b"text");
        want.extend_from_slice(&[0x00, 0x04]);
        want.extend_from_slice(b"boom");
        want.push(TAG_END);
        assert_eq!(bytes, want);
    }

    #[test]
    fn italic_flag_is_a_byte_tag() {
        let bytes = encode_component(&Component::text("x").italic(true));
        // 0a | 01 "italic" 01 | 08 "text" "x" | 00
        let mut want = vec![TAG_COMPOUND, TAG_BYTE];
        want.extend_from_slice(&[0x00, 0x06]);
        want.extend_from_slice(b"italic");
        want.push(0x01);
        want.push(TAG_STRING);
        want.extend_from_slice(&[0x00, 0x04]);
        want.extend_from_slice(b"text");
        want.extend_from_slice(&[0x00, 0x01]);
        want.push(b'x');
        want.push(TAG_END);
        assert_eq!(bytes, want);
    }

    #[test]
    fn extra_children_are_a_homogeneous_compound_list() {
        let bytes = encode_component(
            &Component::text("")
                .push(Component::text("a"))
                .push(Component::colored("b", "gray")),
        );
        assert_eq!(bytes[0], TAG_COMPOUND);
        // The list header: TAG_List entry named "extra", element type compound,
        // i32 count = 2.
        let at = find(&bytes, b"extra").expect("extra entry") + 5;
        assert_eq!(bytes[at], TAG_COMPOUND, "element type");
        assert_eq!(&bytes[at + 1..at + 5], &2i32.to_be_bytes(), "child count");
        // Both children are present, the styled one keeps its colour.
        assert!(find(&bytes, b"gray").is_some());
        assert_eq!(*bytes.last().expect("terminated"), TAG_END);
    }

    #[test]
    fn styling_decides_between_the_string_and_compound_forms() {
        assert_eq!(encode_text("x")[0], TAG_STRING);
        assert_eq!(
            encode_component(&Component::colored("x", "red"))[0],
            TAG_COMPOUND
        );
        assert!(Component::text("x").is_plain());
        assert!(!Component::text("x").italic(false).is_plain());
    }

    #[test]
    fn multibyte_text_is_length_prefixed_in_bytes_not_chars() {
        let bytes = encode_text("привет");
        assert_eq!(&bytes[1..3], &[0x00, 12], "6 cyrillic chars = 12 bytes");
        assert_eq!(bytes.len(), 3 + 12);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }
}
