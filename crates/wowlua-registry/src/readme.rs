//! A package's README as its page shows it: GitHub-flavored Markdown with
//! the design system's markup, as wowlua.dev renders its docs
//! (`lib/markdown.ts`): code blocks become `wl-code` figures (```console a
//! terminal), tables `wl-table` regions, GitHub alerts `wl-callout` boxes.
//!
//! A README is written by whoever publishes, so raw HTML is dropped, links
//! go only to `http(s)`, `mailto` and anchors (relative ones to the
//! package's GitHub repository), and images become links: the page loads
//! nothing from elsewhere.

use crate::highlight::highlight;
use crate::html::esc;
use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

/// The rendered README, and its first wowlua (or Lua) code block: the
/// page's usage example.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Readme {
    pub html: String,
    pub example: Option<String>,
}

/// Where a relative link or image of the README points: the file at the
/// repository's default branch, for a GitHub repository.
fn resolve(url: &str, repository: &str, raw: bool) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
    {
        return Some(url.to_string());
    }
    if url.starts_with('#') {
        return Some(url.to_string());
    }
    if url.contains(':') || url.starts_with("//") {
        return None;
    }
    let repo = repository.trim_end_matches('/').trim_end_matches(".git");
    if !repo.starts_with("https://github.com/") {
        return None;
    }
    let path = url.trim_start_matches("./").trim_start_matches('/');
    if path.split('/').any(|p| p == "..") {
        return None;
    }
    Some(format!(
        "{repo}/{}/HEAD/{path}",
        if raw { "raw" } else { "blob" }
    ))
}

/// A heading's anchor: lowercase words joined by `-`, as GitHub makes them.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if c.is_whitespace() {
            out.push('-');
        }
    }
    out
}

const ALERTS: &[(BlockQuoteKind, &str, &str, &str)] = &[
    (BlockQuoteKind::Note, "note", "i", "note"),
    (BlockQuoteKind::Tip, "tip", "+", "tip"),
    (BlockQuoteKind::Important, "note", "!", "important"),
    (BlockQuoteKind::Warning, "warning", "!", "warning"),
    (BlockQuoteKind::Caution, "error", "×", "caution"),
];

fn code_block(lang: &str, text: &str) -> String {
    let copy = r#"<button class="wl-textbtn" type="button" aria-label="copy">copy</button>"#;
    if lang == "console" {
        let commands: Vec<String> = text
            .lines()
            .filter_map(|l| l.strip_prefix('$'))
            .map(|c| c.trim().to_string())
            .collect();
        let body: Vec<String> = text
            .lines()
            .map(|l| match l.strip_prefix('$') {
                Some(cmd) => format!(
                    r#"<span class="wl-term__prompt">$</span> <span class="wl-term__cmd">{}</span>"#,
                    esc(cmd.trim_start())
                ),
                None => format!(r#"<span class="wl-term__out">{}</span>"#, esc(l)),
            })
            .collect();
        return format!(
            r#"<figure class="wl-term md-term"><div class="wl-term__bar"><span>terminal</span><button class="wl-textbtn" type="button" aria-label="copy" data-copy="{}">copy</button></div><pre class="wl-term__body">{}</pre></figure>"#,
            esc(&commands.join("\n")),
            body.join("\n")
        );
    }
    format!(
        r#"<figure class="wl-code md-code"><div class="wl-code__bar"><span class="wl-code__lang">{}</span>{copy}</div><pre class="wl-code__pre"><code>{}</code></pre></figure>"#,
        esc(if lang.is_empty() { "text" } else { lang }),
        highlight(text, lang)
    )
}

/// Renders `markdown`. A first `#` heading is the package's name, which the
/// page shows itself, so it is left out.
pub fn render(markdown: &str, repository: &str) -> Readme {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut out: Vec<Event> = Vec::new();
    let mut example = None;
    // Open links: whether each was kept (an unsafe one drops its tag, not its text).
    let mut links: Vec<bool> = Vec::new();
    let mut heading: Option<(HeadingLevel, Vec<Event>)> = None;
    let mut code: Option<(String, String)> = None;
    let mut first_heading = true;
    let mut alerts: Vec<bool> = Vec::new();
    for event in Parser::new_ext(markdown, options) {
        if let Some((_, text)) = &mut code {
            match event {
                Event::Text(t) => text.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, text) = code.take().unwrap_or_default();
                    let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
                    if example.is_none() && (lang == "wowlua" || lang == "lua") {
                        example = Some(text.clone());
                    }
                    out.push(Event::Html(code_block(&lang, &text).into()));
                }
                _ => {}
            }
            continue;
        }
        let event = match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
                continue;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level, Vec::new()));
                continue;
            }
            Event::End(TagEnd::Heading(_)) => {
                let Some((level, inner)) = heading.take() else {
                    continue;
                };
                let first = std::mem::replace(&mut first_heading, false);
                if level == HeadingLevel::H1 && first {
                    continue;
                }
                let text: String = inner
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                        _ => None,
                    })
                    .collect();
                // Under the page's own "Readme" heading: one level down.
                let (tag, class) = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => ("h3", "heading-3"),
                    _ => ("h4", "label"),
                };
                out.push(Event::Html(
                    format!(
                        r#"<{tag} class="{class}" id="readme-{}">"#,
                        esc(&slug(&text))
                    )
                    .into(),
                ));
                out.extend(inner);
                out.push(Event::Html(format!("</{tag}>").into()));
                continue;
            }
            Event::Html(_) | Event::InlineHtml(_) => continue,
            Event::Code(t) => {
                Event::Html(format!(r#"<code class="wl-icode">{}</code>"#, esc(&t)).into())
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let href =
                    resolve(&dest_url, repository, false).map(|h| match h.strip_prefix('#') {
                        Some(anchor) => format!("#readme-{anchor}"),
                        None => h,
                    });
                links.push(href.is_some());
                match href {
                    Some(h) => Event::Html(
                        format!(r#"<a href="{}" rel="nofollow noreferrer">"#, esc(&h)).into(),
                    ),
                    None => continue,
                }
            }
            Event::End(TagEnd::Link) => {
                if links.pop().unwrap_or(false) {
                    Event::Html("</a>".into())
                } else {
                    continue;
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let href = resolve(&dest_url, repository, true);
                links.push(href.is_some());
                match href {
                    Some(h) => Event::Html(
                        format!(r#"<a href="{}" rel="nofollow noreferrer">image: "#, esc(&h))
                            .into(),
                    ),
                    None => continue,
                }
            }
            Event::End(TagEnd::Image) => {
                if links.pop().unwrap_or(false) {
                    Event::Html("</a>".into())
                } else {
                    continue;
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                out.push(Event::Html(
                    r#"<div class="wl-table" role="region" tabindex="0">"#.into(),
                ));
                Event::Start(Tag::Table(alignments))
            }
            Event::End(TagEnd::Table) => {
                out.push(Event::End(TagEnd::Table));
                Event::Html("</div>".into())
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                match kind.and_then(|k| ALERTS.iter().find(|a| a.0 == k)) {
                    Some((_, class, glyph, label)) => {
                        alerts.push(true);
                        Event::Html(
                            format!(
                                r#"<aside class="wl-callout wl-callout--{class}" aria-label="{label}"><div class="wl-callout__head"><span class="wl-callout__glyph" aria-hidden="true">{glyph}</span>{label}:</div><div class="wl-callout__body">"#
                            )
                            .into(),
                        )
                    }
                    None => {
                        alerts.push(false);
                        Event::Html(r#"<blockquote class="md-quote">"#.into())
                    }
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if alerts.pop().unwrap_or(false) {
                    Event::Html("</div></aside>".into())
                } else {
                    Event::Html("</blockquote>".into())
                }
            }
            other => other,
        };
        match &mut heading {
            Some((_, inner)) => inner.push(event),
            None => out.push(event),
        }
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, out.into_iter());
    Readme { html, example }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://github.com/a/lib";

    #[test]
    fn design_markup() {
        let r = render(
            "# lib\n\nText with `code`.\n\n## Use it\n\n```console\n$ wowlua add lib\n```\n\n```wowlua\nimport x from \"lib\"\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> [!NOTE]\n> Careful.\n",
            REPO,
        );
        assert!(!r.html.contains("<h1"), "{}", r.html);
        assert!(
            r.html
                .contains(r#"<h3 class="heading-3" id="readme-use-it">Use it</h3>"#),
            "{}",
            r.html
        );
        assert!(r.html.contains(r#"<code class="wl-icode">code</code>"#));
        assert!(r.html.contains(r#"<figure class="wl-term md-term">"#));
        assert!(r.html.contains(r#"data-copy="wowlua add lib""#));
        assert!(r.html.contains(r#"<figure class="wl-code md-code">"#));
        assert!(
            r.html
                .contains(r#"<div class="wl-table" role="region" tabindex="0"><table>"#)
        );
        assert!(
            r.html
                .contains(r#"<aside class="wl-callout wl-callout--note""#),
            "{}",
            r.html
        );
        assert_eq!(r.example.as_deref(), Some("import x from \"lib\""));
    }

    #[test]
    fn nothing_unsafe_or_from_elsewhere() {
        let r = render(
            "<script>alert(1)</script>\n\n[x](javascript:alert(1)) [d](docs/a.md) [up](../x) [s](#use-it)\n\n![logo](logo.png) ![t](https://t.example/p.gif)\n\n<img src=x onerror=alert(1)>\n",
            REPO,
        );
        assert!(!r.html.contains("<script"), "{}", r.html);
        assert!(!r.html.contains("javascript:"), "{}", r.html);
        assert!(!r.html.contains("<img"), "{}", r.html);
        assert!(
            r.html
                .contains(r#"href="https://github.com/a/lib/blob/HEAD/docs/a.md""#),
            "{}",
            r.html
        );
        assert!(
            r.html
                .contains(r#"href="https://github.com/a/lib/raw/HEAD/logo.png""#),
            "{}",
            r.html
        );
        assert!(r.html.contains(r##"href="#readme-use-it""##), "{}", r.html);
        assert!(!r.html.contains("../x"), "{}", r.html);
        // Without a GitHub repository, relative links are text.
        let r = render("[d](docs/a.md)", "https://gitlab.com/a/lib");
        assert_eq!(r.html, "<p>d</p>\n");
    }
}
