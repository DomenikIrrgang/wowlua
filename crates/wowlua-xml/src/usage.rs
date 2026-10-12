//! What a UI XML file defines and uses, for the load order (spec §5.4.2):
//! named frames (globals), templates (`virtual`/`intrinsic` elements), and
//! the globals and templates an element needs when it is created (`mixin`,
//! `secureMixin`, `parent`, handler `function` attributes, `inherits`, inline
//! `<OnLoad>` code). Inline `<Script>` code runs while the file loads.

use quick_xml::Reader;
use quick_xml::events::Event;
use std::ops::Range;

/// A name in the document with the byte range of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub name: String,
    pub span: Range<usize>,
}

/// What creating an element (or loading the file) needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uses {
    /// Globals read when the element is created.
    pub globals: Vec<Name>,
    /// Templates the element inherits.
    pub inherits: Vec<Name>,
    /// Lua code that runs then: `<OnLoad>` bodies and inline `<Script>`
    /// code (byte ranges of the raw text).
    pub scripts: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: Name,
    /// What instantiating the template needs.
    pub uses: Uses,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Named elements created while the file loads (they become globals).
    pub globals: Vec<Name>,
    pub templates: Vec<Template>,
    /// What loading the file needs (its non-template elements).
    pub load: Uses,
}

/// One open element.
struct Open {
    /// Template index the element belongs to (`None`: created at load).
    template: Option<usize>,
    /// The element is `<Scripts>`.
    scripts: bool,
    /// The element is an `<OnLoad>` handler.
    on_load: bool,
    /// The element is an inline `<Script>` (no `file`).
    inline_script: bool,
}

/// Analyzes a document; malformed XML gives an empty result (its errors
/// are reported by [`crate::references`]).
pub fn usage(text: &str) -> Usage {
    let text = &*crate::without_bom(text);
    let mut reader = Reader::from_str(text);
    let mut out = Usage::default();
    let mut stack: Vec<Open> = Vec::new();
    loop {
        let before = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else {
            return Usage::default();
        };
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let tag = &text[before..after];
                let local = element.local_name();
                let local: &str = local.as_ref();
                let local = local.to_string();
                let parent = stack.last();
                let in_scripts = parent.is_some_and(|p| p.scripts);
                let mut template = parent.and_then(|p| p.template);
                let attr = |name: &str| {
                    crate::attribute_value_span(tag, name).map(|r| Name {
                        name: tag[r.clone()].to_string(),
                        span: before + r.start..before + r.end,
                    })
                };
                let flag =
                    |name: &str| attr(name).is_some_and(|v| v.name.eq_ignore_ascii_case("true"));
                if in_scripts {
                    // A script handler: `function="Global"` is read when the
                    // handler is set (as the frame is created).
                    if let Some(f) = attr("function") {
                        uses_of(&mut out, template).globals.push(f);
                    }
                } else if local != "Script" && local != "Include" {
                    if let Some(name) =
                        attr("name").filter(|n| !n.name.contains('$') && !n.name.is_empty())
                    {
                        if flag("virtual") || flag("intrinsic") {
                            out.templates.push(Template {
                                name,
                                uses: Uses::default(),
                            });
                            template = Some(out.templates.len() - 1);
                        } else if template.is_none() {
                            out.globals.push(name);
                        }
                    }
                    for key in ["mixin", "secureMixin", "parent", "relativeTo"] {
                        if let Some(value) = attr(key) {
                            uses_of(&mut out, template)
                                .globals
                                .extend(split_list(&value));
                        }
                    }
                    if let Some(value) = attr("inherits") {
                        uses_of(&mut out, template)
                            .inherits
                            .extend(split_list(&value));
                    }
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(Open {
                        template,
                        scripts: local == "Scripts",
                        on_load: in_scripts && local == "OnLoad",
                        inline_script: local == "Script" && attr("file").is_none(),
                    });
                }
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) => {
                let Some(open) = stack.last() else { continue };
                if open.on_load || (open.inline_script && open.template.is_none()) {
                    let range = if matches!(event, Event::CData(_)) {
                        // `<![CDATA[` … `]]>`
                        before + 9..after.saturating_sub(3).max(before + 9)
                    } else {
                        before..after
                    };
                    let template = open.template;
                    let scripts = &mut uses_of(&mut out, template).scripts;
                    // Text around entity references arrives in pieces.
                    match scripts.last_mut() {
                        Some(last) if last.end == before && !matches!(event, Event::CData(_)) => {
                            last.end = range.end;
                        }
                        _ => scripts.push(range),
                    }
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => return out,
            _ => {}
        }
    }
}

fn uses_of(out: &mut Usage, template: Option<usize>) -> &mut Uses {
    match template {
        Some(t) => &mut out.templates[t].uses,
        None => &mut out.load,
    }
}

/// A comma-separated list (`inherits="A, B"`), without relative names.
fn split_list(value: &Name) -> Vec<Name> {
    let mut out = Vec::new();
    let mut offset = 0;
    for part in value.name.split(',') {
        let trimmed = part.trim();
        let lead = part.len() - part.trim_start().len();
        if !trimmed.is_empty() && !trimmed.contains('$') {
            let start = value.span.start + offset + lead;
            out.push(Name {
                name: trimmed.to_string(),
                span: start..start + trimmed.len(),
            });
        }
        offset += part.len() + 1;
    }
    out
}

/// Lua code from XML text with entities replaced by characters of the same
/// length (`&lt;` → `<` and padding), so byte offsets stay valid.
pub fn script_code(text: &str, range: Range<usize>) -> Vec<u8> {
    let raw = &text.as_bytes()[range];
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'&'
            && let Some(len) = raw[i..].iter().take(10).position(|&b| b == b';')
        {
            let entity = &raw[i + 1..i + len];
            let c = match entity {
                b"lt" => Some(b'<'),
                b"gt" => Some(b'>'),
                b"amp" => Some(b'&'),
                b"quot" => Some(b'"'),
                b"apos" => Some(b'\''),
                _ => None,
            };
            if let Some(c) = c {
                out.push(c);
                out.extend(std::iter::repeat_n(b' ', len));
                i += len + 1;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_definitions_and_uses() {
        let doc = r#"<Ui>
  <Script file="a.lua"/>
  <Frame name="MyTemplate" virtual="true" mixin="TemplateMixin">
    <Scripts><OnLoad>TemplateInit(self)</OnLoad><OnShow>Later()</OnShow></Scripts>
  </Frame>
  <Frame name="MyFrame" parent="UIParent" inherits="MyTemplate, BackdropTemplate" mixin="A,B">
    <Frames><Button name="$parentClose"/></Frames>
    <Scripts><OnClick function="MyFrame_OnClick"/><OnLoad><![CDATA[if a < b then Init(self) end]]></OnLoad></Scripts>
  </Frame>
  <Script>Setup(1 &lt; 2)</Script>
</Ui>"#;
        let u = usage(doc);
        let names = |v: &[Name]| v.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&u.globals), ["MyFrame"]);
        assert_eq!(u.templates.len(), 1);
        assert_eq!(u.templates[0].name.name, "MyTemplate");
        assert_eq!(names(&u.templates[0].uses.globals), ["TemplateMixin"]);
        assert_eq!(
            &doc[u.templates[0].uses.scripts[0].clone()],
            "TemplateInit(self)"
        );
        assert_eq!(
            names(&u.load.globals),
            ["A", "B", "UIParent", "MyFrame_OnClick"]
        );
        assert_eq!(names(&u.load.inherits), ["MyTemplate", "BackdropTemplate"]);
        for n in u.load.globals.iter().chain(&u.load.inherits) {
            assert_eq!(&doc[n.span.clone()], n.name);
        }
        let scripts: Vec<&str> = u.load.scripts.iter().map(|r| &doc[r.clone()]).collect();
        assert_eq!(scripts, ["if a < b then Init(self) end", "Setup(1 &lt; 2)"]);
        let code = script_code(doc, u.load.scripts[1].clone());
        assert_eq!(code, b"Setup(1 <    2)");
    }
}
