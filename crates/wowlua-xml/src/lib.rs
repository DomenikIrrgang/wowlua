//! WoW UI XML (spec §14.8.4): well-formedness checks and the `<Script>` /
//! `<Include>` references that load further files, and what a file defines
//! and uses for the load order ([`usage`]).

use quick_xml::Reader;
use quick_xml::events::Event;
use std::ops::Range;

pub mod usage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Script,
    Include,
}

/// A `file="…"` reference of a `<Script>` or `<Include>` element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlRef {
    pub kind: RefKind,
    /// The attribute value, unchanged (may use `\` separators).
    pub file: String,
    /// Byte range of the attribute value (without quotes) in the document.
    pub span: Range<usize>,
}

impl XmlRef {
    /// The referenced path with `/` separators, without inline directives.
    pub fn normalized(&self) -> String {
        self.path().replace('\\', "/")
    }

    /// The referenced path as written, without inline directives
    /// (`file="Data.lua [AllowLoadGameType tbc]"` → `Data.lua`).
    pub fn path(&self) -> String {
        wowlua_toc::split_conditions(&self.file).0
    }

    /// Inline load directives of the reference, which clients evaluate like
    /// those on TOC file lines.
    pub fn conditions(&self) -> Vec<wowlua_toc::InlineDirective> {
        wowlua_toc::split_conditions(&self.file).1
    }
}

/// A well-formedness error at a byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlError {
    pub message: String,
    pub offset: usize,
}

/// Checks that `text` is well-formed XML and returns its file references in
/// document order.
pub fn references(text: &str) -> Result<Vec<XmlRef>, XmlError> {
    let text = &*without_bom(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut refs = Vec::new();
    let mut depth = 0usize;
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|error| XmlError {
            message: error.to_string(),
            offset: reader.error_position() as usize,
        })?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
                let name = element.local_name();
                let kind = match name.as_ref() {
                    "Script" => Some(RefKind::Script),
                    "Include" => Some(RefKind::Include),
                    _ => None,
                };
                if let Some(kind) = kind {
                    // Validate attribute syntax through quick-xml first.
                    for attribute in element.attributes() {
                        attribute.map_err(|error| XmlError {
                            message: error.to_string(),
                            offset: before,
                        })?;
                    }
                    if let Some(span) = attribute_value_span(&text[before..after], "file") {
                        let span = before + span.start..before + span.end;
                        refs.push(XmlRef {
                            kind,
                            file: text[span.clone()].to_string(),
                            span,
                        });
                    }
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => {
                if depth != 0 {
                    return Err(XmlError {
                        message: "unexpected end of file: unclosed element".into(),
                        offset: text.len(),
                    });
                }
                return Ok(refs);
            }
            _ => {}
        }
    }
}

/// The references of a document that contains nothing but its root element
/// and `<Script file="…"/>` / `<Include file="…"/>` elements (comments and
/// whitespace aside), without inline load directives: such a file can be
/// flattened into the files it loads (spec optimization §15.3.1). `None`
/// for any other document.
pub fn script_only(text: &str) -> Option<Vec<XmlRef>> {
    let text = &*without_bom(text);
    let refs = references(text).ok()?;
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                // Only the root may have content.
                if depth > 0 {
                    return None;
                }
                let _ = e;
                depth += 1;
            }
            Event::Empty(e) => {
                if depth != 1 || !matches!(e.local_name().as_ref(), "Script" | "Include") {
                    return None;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Text(t) => {
                if !t.as_ref().trim().is_empty() {
                    return None;
                }
            }
            Event::CData(_) => return None,
            Event::Eof => break,
            _ => {}
        }
    }
    let plain = refs.iter().all(|r| r.conditions().is_empty());
    plain.then_some(refs)
}

/// The document with a leading byte order mark (which the client accepts)
/// replaced by spaces, so that byte offsets stay those of the file.
pub(crate) fn without_bom(text: &str) -> std::borrow::Cow<'_, str> {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => format!("   {rest}").into(),
        None => text.into(),
    }
}

/// Byte range of an attribute's value inside the text of one tag.
fn attribute_value_span(tag: &str, name: &str) -> Option<Range<usize>> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while let Some(found) = tag[i..].find(name) {
        let start = i + found;
        let mut j = start + name.len();
        let boundary = start > 0 && bytes[start - 1].is_ascii_whitespace();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if boundary && j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if let Some(&quote) = bytes.get(j).filter(|b| **b == b'"' || **b == b'\'') {
                let value_start = j + 1;
                let len = tag[value_start..].find(quote as char)?;
                return Some(value_start..value_start + len);
            }
        }
        i = start + name.len();
    }
    None
}

/// Byte ranges of the packager's `<!--@debug@-->` … `<!--@end-debug@-->`
/// blocks in a document.
pub fn debug_blocks(text: &str) -> Vec<Range<usize>> {
    const START: &str = "<!--@debug@-->";
    const END: &str = "<!--@end-debug@-->";
    let mut blocks = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(START) {
        let start = from + i;
        let Some(j) = text[start..].find(END) else {
            break;
        };
        let end = start + j + END.len();
        blocks.push(start..end);
        from = end;
    }
    blocks
}

/// The byte range of the element holding a reference (`<Script … />`).
pub fn element_range(text: &str, reference: &XmlRef) -> Range<usize> {
    let start = text[..reference.span.start]
        .rfind('<')
        .unwrap_or(reference.span.start);
    let end = text[reference.span.end..]
        .find('>')
        .map_or(reference.span.end, |i| reference.span.end + i + 1);
    start..end
}

/// Replaces reference values. `replacements` pairs each reference with its
/// new value; the rest of the document is kept byte for byte.
pub fn rewrite(text: &str, replacements: &[(&XmlRef, String)]) -> String {
    let mut ordered: Vec<&(&XmlRef, String)> = replacements.iter().collect();
    ordered.sort_by_key(|(r, _)| std::cmp::Reverse(r.span.start));
    let mut out = text.to_string();
    for (reference, value) in ordered {
        out.replace_range(reference.span.clone(), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_script_only_documents() {
        let flat = "<Ui xmlns=\"x\">\n<!-- libs -->\n<Script file=\"a.lua\"/>\n<Include file=\"b\\b.xml\"/>\n</Ui>";
        let refs = script_only(flat).unwrap();
        assert_eq!(refs.len(), 2);
        assert!(script_only("<Ui><Frame name=\"X\"/></Ui>").is_none());
        assert!(script_only("<Ui><Script>print(1)</Script></Ui>").is_none());
        assert!(script_only("<Ui><Script file=\"a.lua [AllowLoadGameType tbc]\"/></Ui>").is_none());
    }

    const DOC: &str = r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
  <Script file="core.lua"/>
  <Include file='Libs\LibStub\LibStub.xml' />
  <Frame name="X"><Scripts><OnLoad>print("file=nope")</OnLoad></Scripts></Frame>
  <Script   file = "ui\window.wowlua"></Script>
</Ui>"#;

    #[test]
    fn finds_references_with_spans() {
        let refs = references(DOC).unwrap();
        let files: Vec<&str> = refs.iter().map(|r| r.file.as_str()).collect();
        assert_eq!(
            files,
            [
                "core.lua",
                "Libs\\LibStub\\LibStub.xml",
                "ui\\window.wowlua"
            ]
        );
        assert_eq!(refs[1].kind, RefKind::Include);
        assert_eq!(refs[1].normalized(), "Libs/LibStub/LibStub.xml");
        for r in &refs {
            assert_eq!(&DOC[r.span.clone()], r.file);
        }
    }

    #[test]
    fn rewrites_only_the_values() {
        let refs = references(DOC).unwrap();
        let out = rewrite(DOC, &[(&refs[2], "ui\\window.lua".into())]);
        assert!(out.contains(r#"<Script   file = "ui\window.lua"></Script>"#));
        assert!(out.contains(r#"<Script file="core.lua"/>"#));
    }

    #[test]
    fn separates_inline_directives_from_paths() {
        let text = r#"<Ui><Include file="Data\TBC.lua [AllowLoadGameType: tbc]"/></Ui>"#;
        let refs = references(text).unwrap();
        assert_eq!(refs[0].path(), "Data\\TBC.lua");
        assert_eq!(refs[0].normalized(), "Data/TBC.lua");
        assert_eq!(refs[0].conditions()[0].values, ["tbc"]);
    }

    #[test]
    fn reports_malformed_xml() {
        let error = references("<Ui><Frame></Ui>").unwrap_err();
        assert!(
            error.message.contains("Frame") || error.message.contains("Ui"),
            "{}",
            error.message
        );
        assert!(references("<Ui>").is_err());
    }

    #[test]
    fn skips_a_byte_order_mark() {
        let text = "\u{feff}<?xml version=\"1.0\"?><Ui>\n\t<Include file='A\\B.xml'/>\n</Ui>";
        let refs = references(text).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(&text[refs[0].span.clone()], "A\\B.xml");
    }
}
