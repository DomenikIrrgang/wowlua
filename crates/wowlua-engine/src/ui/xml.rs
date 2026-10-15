//! UI XML as an element tree: what the loader walks.

use std::rc::Rc;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// One element: its tag, attributes in order, children, and its text (a
/// `<Script>` body), with the line it starts on.
#[derive(Debug, Default)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Rc<Element>>,
    pub text: String,
    pub line: usize,
}

impl Element {
    /// An attribute's value; names are case-insensitive, as the client's.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// A boolean attribute (`"true"`, any case).
    pub fn flag(&self, name: &str) -> bool {
        self.attr(name)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }

    /// A number attribute.
    pub fn number(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }

    /// The first child with `tag` (case-insensitive).
    pub fn child(&self, tag: &str) -> Option<&Rc<Element>> {
        self.children
            .iter()
            .find(|c| c.tag.eq_ignore_ascii_case(tag))
    }
}

fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn element(text: &str, start: &BytesStart, offset: usize) -> Element {
    let attrs = start
        .attributes()
        .flatten()
        .map(|a| {
            let local = a.key.local_name();
            let key: &str = local.as_ref();
            let value = a
                .normalized_value(XmlVersion::Implicit1_0)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| a.value.to_string());
            (key.to_string(), value)
        })
        .collect();
    Element {
        tag: {
            let local = start.local_name();
            let tag: &str = local.as_ref();
            tag.to_string()
        },
        attrs,
        children: Vec::new(),
        text: String::new(),
        line: line_of(text, offset),
    }
}

/// Parses a UI XML document into its root element.
pub fn parse(text: &str) -> Result<Rc<Element>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Element> = vec![Element {
        tag: "#document".into(),
        ..Element::default()
    }];
    loop {
        let offset = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| format!("line {}: {e}", line_of(text, offset)))?;
        match event {
            Event::Start(s) => stack.push(element(text, &s, offset)),
            Event::Empty(s) => {
                let e = element(text, &s, offset);
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(Rc::new(e));
                }
            }
            Event::End(_) => {
                let done = stack.pop().ok_or("unbalanced end tag")?;
                let parent = stack.last_mut().ok_or("unbalanced end tag")?;
                parent.children.push(Rc::new(done));
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.into_inner());
                }
            }
            Event::GeneralRef(r) => {
                let c = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => match &*r.into_inner() {
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "amp" => Some('&'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    },
                };
                if let (Some(c), Some(top)) = (c, stack.last_mut()) {
                    top.text.push(c);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut doc = stack.pop().ok_or("empty document")?;
    if !stack.is_empty() {
        return Err("unclosed element".into());
    }
    doc.children
        .pop()
        .ok_or_else(|| "no root element".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_script_text() {
        let root = parse(
            "<Ui xmlns=\"http://www.blizzard.com/wow/ui/\">\n<Frame name=\"A\" Hidden=\"true\">\n<Scripts><OnLoad>print(1 &lt; 2)</OnLoad></Scripts>\n</Frame>\n<Script file=\"x.lua\"/>\n</Ui>",
        )
        .unwrap();
        assert_eq!(root.tag, "Ui");
        let frame = &root.children[0];
        assert_eq!(frame.attr("name"), Some("A"));
        assert!(frame.flag("hidden"));
        assert_eq!(frame.line, 2);
        let onload = frame.child("Scripts").unwrap().child("OnLoad").unwrap();
        assert_eq!(onload.text, "print(1 < 2)");
        assert_eq!(root.children[1].attr("file"), Some("x.lua"));
    }
}
