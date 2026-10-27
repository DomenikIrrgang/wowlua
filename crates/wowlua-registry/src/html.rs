//! The frame every page shares, from the "wowlua registry" design canvas:
//! wowlua.dev's header (with the registry as its current section), the
//! registry bar, and wowlua.dev's footer. The design system's markup and
//! classes are the website's (`assets/`).

use crate::assets;

pub const SITE: &str = "https://wowlua.dev";
pub const REPO: &str = "https://github.com/DomenikIrrgang/wowlua";
pub const DISCORD: &str = "https://discord.gg/Dn795czPYk";

const LOGO: &str = include_str!("../assets/logo.svg");
const MARK: &str = include_str!("../assets/mark.svg");
const GITHUB: &str = include_str!("../assets/github.svg");
const DISCORD_ICON: &str = include_str!("../assets/discord.svg");

/// Escapes text for HTML.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Escapes a query-string value.
pub fn query_value(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The registry bar's tab that is current.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Packages,
    Account,
    None,
}

/// What a page puts in the frame.
pub struct Frame<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub tab: Tab,
    /// The signed-in user's GitHub login.
    pub user: Option<&'a str>,
}

/// The wowlua version the header shows (`v0.13.1`), as wowlua.dev does:
/// this server's.
fn version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

const NAV: &[(&str, &str)] = &[
    ("home", SITE),
    ("docs", "https://wowlua.dev/docs/guide/getting-started"),
    ("api", "https://wowlua.dev/api"),
    ("registry", "/"),
    ("playground", "https://wowlua.dev/playground"),
    ("blog", "https://wowlua.dev/blog"),
];

const THEME_SCRIPT: &str = r#"try{var t=localStorage.getItem("wl-theme");if(t==="dark"||t==="light")document.documentElement.setAttribute("data-theme",t)}catch(e){}"#;

/// A whole page: `main` is the page's `<main>` element.
pub fn page(frame: &Frame, main: &str) -> String {
    let v = assets::version();
    let stylesheets: String = ["tokens", "design-system", "chrome", "registry"]
        .iter()
        .map(|f| format!("<link rel=\"stylesheet\" href=\"/assets/{f}.css?v={v}\">"))
        .collect();
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{description}">
<meta name="theme-color" content="#ffb547">
<script>{THEME_SCRIPT}</script>
{stylesheets}
<script src="/assets/registry.js?v={v}" defer></script>
</head>
<body>
<div class="rg-page">
{header}
{bar}
{main}
{footer}
</div>
{menu}
</body>
</html>
"##,
        title = esc(&if frame.title.is_empty() {
            "wowlua registry".to_string()
        } else {
            format!("{} · wowlua registry", frame.title)
        }),
        description = esc(frame.description),
        header = header(),
        bar = bar(frame),
        footer = footer(),
        menu = menu(),
    )
}

fn theme_toggle() -> String {
    const ICONS: &[(&str, &str)] = &[
        (
            "system",
            r#"<rect x="1.75" y="2.75" width="12.5" height="8.5"/><path d="M5 14h6"/>"#,
        ),
        (
            "dark",
            r#"<path d="M13.5 9.5A6 6 0 0 1 6.5 2.5a6 6 0 1 0 7 7Z"/>"#,
        ),
        (
            "light",
            r#"<circle cx="8" cy="8" r="3"/><path d="M8 1v2M8 13v2M1 8h2M13 8h2M3 3l1.4 1.4M11.6 11.6 13 13M3 13l1.4-1.4M11.6 4.4 13 3"/>"#,
        ),
    ];
    let buttons: String = ICONS
        .iter()
        .map(|(choice, icon)| {
            format!(
                r#"<button class="wl-theme-toggle__opt" type="button" role="radio" aria-checked="{}" aria-label="{choice} theme" title="{choice}" data-theme-choice="{choice}"><svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">{icon}</svg><span class="wl-theme-toggle__label">{choice}</span></button>"#,
                *choice == "system"
            )
        })
        .collect();
    format!(
        r#"<div class="wl-theme-toggle wl-theme-toggle--icons" role="radiogroup" aria-label="Theme">{buttons}</div>"#
    )
}

fn header() -> String {
    let links: String = NAV
        .iter()
        .map(|(label, href)| {
            let current = if *label == "registry" {
                r#" aria-current="page""#
            } else {
                ""
            };
            format!(r#"<a class="wl-site-header__link" href="{href}"{current}>{label}</a>"#)
        })
        .collect();
    format!(
        r#"<header class="wl-site-header" data-site-header>
  <div class="wl-site-header__inner">
    <a class="wl-site-header__brand" href="{SITE}" aria-label="wowlua home">{LOGO}</a>
    <span class="wl-site-header__version">{version}</span>
    <nav class="wl-site-header__nav" aria-label="Main">{links}</nav>
    <div class="wl-site-header__end">
      {toggle}
      <a class="wl-site-header__search" href="{SITE}/?search" aria-label="search the docs"><svg class="wl-site-header__search-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="square" aria-hidden="true"><circle cx="7" cy="7" r="4.75"/><path d="M10.5 10.5 14.25 14.25"/></svg><span>search docs</span><kbd class="wl-kbd">ctrl k</kbd></a>
      <a class="wl-site-header__icon" href="{REPO}" aria-label="wowlua on GitHub">{GITHUB}</a>
      <a class="wl-site-header__icon" href="{DISCORD}" aria-label="wowlua on Discord">{DISCORD_ICON}</a>
      <button class="wl-site-header__icon wl-site-header__menu" type="button" aria-label="open menu" aria-expanded="false" aria-controls="rg-menu" data-menu-open><svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M2 4h12M2 8h12M2 12h12"/></svg></button>
    </div>
  </div>
</header>"#,
        version = version(),
        toggle = theme_toggle(),
    )
}

fn bar(frame: &Frame) -> String {
    let tab = |label: &str, href: &str, current: bool| {
        format!(
            r#"<a class="rg-bar__tab label" href="{href}"{}>{label}</a>"#,
            if current {
                r#" aria-current="page""#
            } else {
                ""
            }
        )
    };
    let end = match frame.user {
        Some(login) => format!(
            r#"<a class="rg-bar__user label" href="/account"{}><span class="rg-muted">@</span>{}</a>"#,
            if frame.tab == Tab::Account {
                r#" aria-current="page""#
            } else {
                ""
            },
            esc(login)
        ),
        None => r#"<a class="wl-btn wl-btn--secondary wl-btn--small" href="/login">sign in</a>"#
            .to_string(),
    };
    format!(
        r#"<div class="rg-bar">
  <div class="rg-bar__inner">
    <span class="rg-bar__site label">registry.wowlua.dev</span>
    <nav class="rg-bar__nav" aria-label="Registry">{}{}</nav>
    <div class="rg-bar__end">{end}</div>
  </div>
</div>"#,
        tab("packages", "/", frame.tab == Tab::Packages),
        tab(
            "publish a library ↗",
            "https://wowlua.dev/docs/guide/publishing",
            false
        ),
    )
}

const LEARN: &[(&str, &str)] = &[
    (
        "getting started",
        "https://wowlua.dev/docs/guide/getting-started",
    ),
    ("language tour", "https://wowlua.dev/docs/language/tour"),
    (
        "standard library",
        "https://wowlua.dev/docs/stdlib/overview",
    ),
    (
        "libraries and dependencies",
        "https://wowlua.dev/docs/guide/packages",
    ),
    ("publishing", "https://wowlua.dev/docs/guide/publishing"),
];
const REFERENCE: &[(&str, &str)] = &[
    ("wow api", "https://wowlua.dev/api"),
    ("registry", "/"),
    ("commands", "https://wowlua.dev/docs/guide/commands"),
    ("wowlua.json", "https://wowlua.dev/docs/reference/manifest"),
    (
        "errors and warnings",
        "https://wowlua.dev/docs/reference/diagnostics",
    ),
];
const PROJECT: &[(&str, &str)] = &[
    ("github ↗", REPO),
    ("discord ↗", DISCORD),
    ("changelog", "https://wowlua.dev/changelog"),
    ("blog", "https://wowlua.dev/blog"),
    ("showcase", "https://wowlua.dev/showcase"),
];

fn links(list: &[(&str, &str)]) -> String {
    list.iter()
        .map(|(label, href)| format!(r#"<li><a href="{href}">{label}</a></li>"#))
        .collect()
}

fn footer() -> String {
    let col = |title: &str, list: &[(&str, &str)]| {
        format!(
            r#"<nav class="wl-site-footer__col" aria-label="{}"><h2>{title}</h2><ul>{}</ul></nav>"#,
            capitalized(title),
            links(list)
        )
    };
    let fold = |title: &str, list: &[(&str, &str)]| {
        format!(
            r##"<details class="m-footer__section"><summary class="label"><span><span aria-hidden="true"># </span>{title}</span><span class="m-footer__sign" aria-hidden="true"></span></summary><ul class="body-s">{}</ul></details>"##,
            links(list)
        )
    };
    format!(
        r#"<footer class="wl-site-footer" style="font-family: var(--font-mono);">
  <div class="wl-site-footer__inner">
    <div class="wl-site-footer__brand">{MARK}<p>A language, compiler, framework and package manager for World of Warcraft AddOns.</p></div>
    {}{}{}
    <nav class="m-footer" aria-label="Footer">{}{}{}</nav>
    <div class="wl-site-footer__base">
      <span>MIT licence · fonts: Martian Mono, IBM Plex Mono (SIL OFL)</span>
      <span>Not affiliated with Blizzard Entertainment. World of Warcraft is a trademark of Blizzard Entertainment, Inc.</span>
    </div>
  </div>
</footer>"#,
        col("learn", LEARN),
        col("reference", REFERENCE),
        col("project", PROJECT),
        fold("learn", LEARN),
        fold("reference", REFERENCE),
        fold("project", PROJECT),
    )
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The phone menu (wowlua.dev's), opened from the header's menu button.
fn menu() -> String {
    let nav: String = NAV
        .iter()
        .map(|(label, href)| {
            format!(
                r#"<li><a href="{href}"{}><span><span class="m-menu__mark" aria-hidden="true">&gt; </span>{label}</span><span class="m-menu__arrow" aria-hidden="true">→</span></a></li>"#,
                if *label == "registry" {
                    r#" aria-current="page""#
                } else {
                    ""
                }
            )
        })
        .collect();
    let group = |title: &str, list: &[(&str, &str)]| {
        let items: String = list
            .iter()
            .map(|(label, href)| format!(r#"<a class="body-s" href="{href}">{label}</a>"#))
            .collect();
        format!(r#"<div class="m-menu__group"><h2 class="label">{title}</h2>{items}</div>"#)
    };
    let themes: String = ["system", "dark", "light"]
        .iter()
        .map(|c| {
            format!(
                r#"<button type="button" role="radio" aria-checked="{}" data-theme-choice="{c}">{c}</button>"#,
                *c == "system"
            )
        })
        .collect();
    format!(
        r#"<div class="m-menu" id="rg-menu" role="dialog" aria-modal="true" aria-label="Site menu" hidden>
  <div class="m-menu__bar"><a class="m-menu__home" href="{SITE}" aria-label="wowlua home">{MARK}</a><span class="label">menu</span><button class="m-menu__close" type="button" aria-label="close menu" data-menu-close>×</button></div>
  <div class="m-menu__search"><a href="{SITE}/?search"><span aria-hidden="true">/</span>search docs</a></div>
  <nav class="m-menu__nav" aria-label="Main"><ul>{nav}</ul></nav>
  <div class="m-menu__groups">{}{}</div>
  <div class="m-menu__foot">
    <div class="m-menu__theme"><span class="caption">theme</span><div role="radiogroup" aria-label="Theme">{themes}</div></div>
    <div class="m-menu__links"><div><a class="label" href="{REPO}">github ↗</a><a class="label" href="{DISCORD}">discord ↗</a></div><span class="caption">wowlua {version} · MIT</span></div>
  </div>
</div>"#,
        group(
            "guide",
            &[
                ("installation", "https://wowlua.dev/docs/guide/install"),
                (
                    "libraries and dependencies",
                    "https://wowlua.dev/docs/guide/packages"
                ),
                ("publishing", "https://wowlua.dev/docs/guide/publishing"),
            ]
        ),
        group(
            "reference",
            &[
                ("commands", "https://wowlua.dev/docs/guide/commands"),
                ("wowlua.json", "https://wowlua.dev/docs/reference/manifest"),
                (
                    "errors and warnings",
                    "https://wowlua.dev/docs/reference/diagnostics"
                ),
            ]
        ),
        version = version().trim_start_matches('v'),
    )
}

/// A `$ command` box with a copy button.
pub fn copy_cmd(command: &str, label: &str) -> String {
    format!(
        r#"<div class="wl-copy-cmd"><span class="wl-copy-cmd__prompt" aria-hidden="true">$</span><code class="wl-copy-cmd__text">{}</code><button class="wl-copy-cmd__btn" type="button" aria-label="{}">copy</button></div>"#,
        esc(command),
        esc(label)
    )
}

/// `YYYY-MM-DD` of a time in seconds.
pub fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_escapes() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_792_972_800), "2026-10-26");
        assert_eq!(
            esc("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(query_value("lfg chat/ü"), "lfg+chat%2F%C3%BC");
    }
}
