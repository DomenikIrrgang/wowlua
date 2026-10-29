//! The registry's pages, from the "wowlua registry" design canvas: the
//! packages with search and filters, a package's page, sign-in with GitHub,
//! the account page where a user creates and revokes API tokens, yanks
//! versions and manages the owners of their packages, and the admins'
//! reserved names.

use crate::api::{ApiError, blocking, word_match};
use crate::html::{self, Frame, Tab, copy_cmd, date, esc, query_value};
use crate::store::{Reserved, Token, VersionMeta, same_login};
use crate::{Shared, auth, readme};
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use std::collections::HashMap;
use wowlua_pkg::registry::IndexEntry;

const SESSION_COOKIE: &str = "wlr_session";
const STATE_COOKIE: &str = "wlr_state";

/// Where someone claims a reserved name: an issue form in wowlua's
/// repository, filled in with the name.
pub fn claim_url(name: &str) -> String {
    format!(
        "{}/issues/new?template=reserved-name.yml&title={}&name={}",
        html::REPO,
        query_value(&format!("Reserved name: {name}")),
        query_value(name)
    )
}

const KINDS: &[&str] = &["library", "lua-library", "types"];

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn set_cookie(app: &Shared, name: &str, value: &str, max_age: u64) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if app.config.secure() { "; Secure" } else { "" }
    )
}

/// The signed-in user's login, if any (pages anyone may see).
fn viewer(app: &Shared, headers: &HeaderMap) -> Option<String> {
    cookie(headers, SESSION_COOKIE)
        .and_then(|id| app.sessions.get(id))
        .map(|s| s.login)
}

fn render(frame: &Frame, main: &str) -> Html<String> {
    Html(html::page(frame, main))
}

/// The registry's address without its scheme (`registry.wowlua.dev`).
fn host(app: &Shared) -> String {
    let url = &app.config.public_url;
    url.split_once("://")
        .map_or(url.as_str(), |(_, h)| h)
        .to_string()
}

/// An error page, shaped like the compiler's diagnostics.
struct Problem<'a> {
    status: StatusCode,
    message: String,
    /// The page's path (`/packages/x`).
    path: &'a str,
    help: String,
    /// The first button.
    action: (&'a str, String),
}

fn problem(app: &Shared, user: Option<&str>, p: Problem) -> Response {
    let main = format!(
        r#"<main class="rg-main rg-main--narrow rg-error">
<pre class="wl-diag"><span class="wl-diag__error">error[{code}]</span><span class="wl-diag__msg">: {message}</span>
   <span class="wl-diag__gutter">┌─</span> {host}{path}
   <span class="wl-diag__gutter">│</span>
   <span class="wl-diag__gutter">=</span> <span class="wl-diag__help">help:</span> {help}</pre>
<div class="rg-error-actions"><a class="wl-btn wl-btn--primary" href="{href}">{label}</a><a class="wl-btn wl-btn--secondary" href="/">all packages</a></div>
</main>"#,
        code = p.status.as_u16(),
        message = esc(&p.message),
        host = esc(&host(app)),
        path = esc(p.path),
        help = esc(&p.help),
        href = esc(&p.action.1),
        label = esc(p.action.0),
    );
    let frame = Frame {
        title: p.status.canonical_reason().unwrap_or("Error"),
        description: "",
        tab: Tab::None,
        user,
    };
    (p.status, render(&frame, &main)).into_response()
}

/// A form's failure, back to the account page.
fn message(app: &Shared, user: Option<&str>, text: &str, status: StatusCode) -> Response {
    problem(
        app,
        user,
        Problem {
            status,
            message: text.to_string(),
            path: "/account",
            help: "nothing was changed; go back and try again".into(),
            action: ("your account", "/account".into()),
        },
    )
}

/// The newest version not yanked (else the newest).
fn newest(entries: &[IndexEntry]) -> Option<&IndexEntry> {
    entries
        .iter()
        .filter(|e| !e.yanked)
        .max_by_key(|e| e.version())
        .or_else(|| entries.iter().max_by_key(|e| e.version()))
}

/// What `wowlua add` asks for: `^1.2`, or `^0.1` for 0.1.x.
fn requirement(version: &str) -> String {
    match semver::Version::parse(version) {
        Ok(v) if v.major > 0 => format!("^{}.{}", v.major, v.minor),
        Ok(v) if v.minor > 0 => format!("^0.{}", v.minor),
        Ok(v) => format!("^0.0.{}", v.patch),
        Err(_) => version.to_string(),
    }
}

fn badges(flavors: &[String]) -> String {
    flavors
        .iter()
        .map(|f| format!(r#"<span class="wl-badge">{}</span>"#, esc(f)))
        .collect()
}

// ---- packages ----------------------------------------------------------------

/// A query's values for `key`, in order.
fn values<'a>(query: &'a [(String, String)], key: &'a str) -> impl Iterator<Item = &'a str> {
    query
        .iter()
        .filter(move |(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// A package as the list shows it.
struct Listed {
    name: String,
    vers: String,
    wowlua: String,
    meta: VersionMeta,
}

/// `GET /?q&kind&flavor`: the packages, searched and filtered.
pub async fn home(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<Vec<(String, String)>>,
) -> Html<String> {
    let user = viewer(&app, &headers);
    let all = blocking({
        let app = app.clone();
        move || {
            app.store
                .packages()
                .into_iter()
                .filter_map(|name| {
                    let entries = app.store.entries(&name);
                    let e = entries
                        .iter()
                        .filter(|e| !e.yanked)
                        .max_by_key(|e| e.version())?;
                    let meta = app.store.version_meta(&name, &e.vers)?;
                    Some(Listed {
                        vers: e.vers.clone(),
                        wowlua: e.wowlua.clone().unwrap_or_else(|| "any".into()),
                        name,
                        meta,
                    })
                })
                .collect::<Vec<_>>()
        }
    })
    .await;
    let get = |key: &'static str| values(&query, key);
    let q = get("q").next().unwrap_or("").trim().to_string();
    let words: Vec<String> = q
        .split_whitespace()
        .take(10)
        .map(|w| w.to_lowercase().chars().take(64).collect())
        .collect();
    // A submitted form says which kinds are checked (maybe none); a link
    // without `f` has every kind.
    let submitted = get("f").next().is_some();
    let kinds: Vec<&str> = if submitted {
        get("kind").filter(|k| KINDS.contains(k)).collect()
    } else {
        KINDS.to_vec()
    };
    let flavors: Vec<&str> = get("flavor").collect();
    // The flavor table's flavors, in its order, then any other a package names.
    let mut flavor_list: Vec<String> = wowlua_flavors::table()
        .flavors
        .iter()
        .map(|f| f.id.clone())
        .collect();
    for p in &all {
        for f in &p.meta.flavors {
            if !flavor_list.contains(f) {
                flavor_list.push(f.clone());
            }
        }
    }
    let filtered = kinds.len() < KINDS.len() || !flavors.is_empty();
    let total = all.len();
    let mut shown: Vec<(u32, String, Listed)> = all
        .into_iter()
        .filter(|p| kinds.contains(&p.meta.kind.as_str()))
        .filter(|p| {
            flavors.is_empty() || p.meta.flavors.iter().any(|f| flavors.contains(&f.as_str()))
        })
        .filter_map(|p| {
            if words.is_empty() {
                let foot = format!("{} · wowlua {}", date(p.meta.published), p.wowlua);
                return Some((0, foot, p));
            }
            let mut score = 0;
            let mut parts: Vec<String> = Vec::new();
            for w in &words {
                let (s, [named, keyword, described]) =
                    word_match(w, &p.name, &p.meta.keywords, &p.meta.description);
                if s == 0 {
                    return None;
                }
                score += s;
                let mut add = |part: String| {
                    if !parts.contains(&part) {
                        parts.push(part);
                    }
                };
                if named {
                    add("name".into());
                }
                if keyword {
                    add(format!("keyword {w}"));
                }
                if described {
                    add("description".into());
                }
            }
            Some((score, format!("matches the {}", parts.join(" · ")), p))
        })
        .collect();
    if words.is_empty() {
        shown.sort_by(|a, b| {
            b.2.meta
                .published
                .cmp(&a.2.meta.published)
                .then_with(|| a.2.name.cmp(&b.2.name))
        });
    } else {
        shown.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.name.cmp(&b.2.name)));
    }
    let n = shown.len();
    let quoted = format!("“{q}”");
    let heading = if !words.is_empty() {
        if n == 1 {
            format!("1 package matches {quoted}")
        } else {
            format!("{n} packages match {quoted}")
        }
    } else if filtered {
        format!("{n} of {total} packages")
    } else {
        "All packages".to_string()
    };
    let cards: String = shown
        .iter()
        .map(|(_, foot, p)| {
            format!(
                r#"<article class="wl-pkg"><div class="wl-pkg__head"><h3 class="wl-pkg__name"><a href="/packages/{name}">{name}</a></h3><span class="wl-pkg__version">{vers}</span></div><div class="wl-badge-row"><span class="wl-badge wl-badge--release">{kind}</span>{flavors}</div><p class="wl-pkg__desc">{desc}</p><div class="wl-pkg__foot"><span><span class="rg-pkg__desk">{foot}</span><span class="rg-pkg__phone">{date}</span></span><span class="wl-pkg__cmd rg-pkg__desk">wowlua add {name}</span><span class="wl-pkg__cmd rg-pkg__phone">wowlua {wowlua}</span></div></article>"#,
                name = esc(&p.name),
                vers = esc(&p.vers),
                kind = esc(&p.meta.kind),
                flavors = badges(&p.meta.flavors),
                desc = esc(&p.meta.description),
                foot = esc(foot),
                date = date(p.meta.published),
                wowlua = esc(&p.wowlua),
            )
        })
        .collect();
    let empty = if n == 0 {
        let what = if words.is_empty() {
            "these filters".to_string()
        } else if filtered {
            format!("{quoted} with these filters")
        } else {
            quoted.clone()
        };
        format!(
            r#"<div class="rg-empty"><p class="body-s" style="color: var(--ink);">No package matches {}.</p><p class="body-s">A library that isn&#39;t on the registry yet can still be a <code class="wl-icode">git</code> or <code class="wl-icode">path</code> dependency of your AddOn. <a href="https://wowlua.dev/docs/guide/packages">Dependency sources →</a></p></div>"#,
            esc(&what)
        )
    } else {
        String::new()
    };
    let check = |name: &str, value: &str, on: bool| {
        format!(
            r#"<label class="wl-check"><input class="wl-check__box" type="checkbox" form="search" name="{name}" value="{v}"{} data-autosubmit> {v}</label>"#,
            if on { " checked" } else { "" },
            v = esc(value)
        )
    };
    let kind_boxes: String = KINDS
        .iter()
        .map(|k| check("kind", k, kinds.contains(k)))
        .collect();
    let flavor_boxes: String = flavor_list
        .iter()
        .map(|f| check("flavor", f, flavors.contains(&f.as_str())))
        .collect();
    let clear = if filtered {
        format!(
            r#"<a class="wl-btn wl-btn--ghost wl-btn--small" href="/{}" style="justify-self: start;">clear filters</a>"#,
            if q.is_empty() {
                String::new()
            } else {
                format!("?q={}", query_value(&q))
            }
        )
    } else {
        String::new()
    };
    let picked = if kinds.len() == 1 { kinds[0] } else { "" };
    let kind_options: String = std::iter::once(("", "all kinds"))
        .chain(KINDS.iter().map(|k| (*k, *k)))
        .map(|(v, label)| {
            format!(
                r#"<option value="{v}"{}>{label}</option>"#,
                if v == picked { " selected" } else { "" }
            )
        })
        .collect();
    let newest_name = shown
        .iter()
        .map(|(_, _, p)| p)
        .max_by_key(|p| p.meta.published)
        .map(|p| p.name.clone());
    let example = newest_name
        .map(|n| copy_cmd(&format!("wowlua add {n}"), "copy add command"))
        .unwrap_or_default();
    let main = format!(
        r#"<main class="rg-main rg-main--home">
<section class="rg-hero">
  <div class="rg-hero__text">
    <h1 class="display-l">Libraries for your AddOn</h1>
    <p class="body-l rg-muted">Typed wowlua libraries you depend on by version. wowlua locks each one with its checksum and checks your calls into it like your own code.</p>
  </div>
  <form class="rg-search" id="search" role="search" action="/" method="get">
    <input type="hidden" name="f" value="1">
    <div class="wl-field">
      <label class="wl-field__label" for="q">search packages</label>
      <div class="wl-input"><span class="wl-input__prefix" aria-hidden="true">/</span><input class="wl-input__control" id="q" name="q" type="search" placeholder="name, description or keyword" value="{q}" aria-describedby="q-help"></div>
      <p class="wl-field__help" id="q-help">Same results as <code class="wl-icode">wowlua search &lt;words&gt;</code>: names first, then keywords, then descriptions.</p>
    </div>
    <button class="wl-btn wl-btn--primary" type="submit">search</button>
  </form>
  {example}
</section>
<div class="rg-split">
  <aside class="rg-filters" aria-label="Filters">
    <fieldset class="wl-fieldset"><legend>kind</legend>{kind_boxes}</fieldset>
    <fieldset class="wl-fieldset"><legend>flavor</legend>{flavor_boxes}<span class="wl-check__help">None checked: every flavor.</span></fieldset>
    {clear}
  </aside>
  <section class="rg-results" aria-labelledby="results">
    <div class="rg-row"><h2 class="heading-2" id="results" style="margin: 0;">{heading}</h2><span class="caption rg-muted">{order}</span></div>
    <div class="wl-select wl-select--small rg-kind"><label class="wl-field__label rg-hide" for="kind-picker">kind</label><select class="wl-select__control" id="kind-picker" data-kind-picker="search">{kind_options}</select></div>
    <div class="wl-pkgs">{cards}</div>
    {empty}
  </section>
</div>
</main>"#,
        q = esc(&q),
        heading = esc(&heading),
        order = if words.is_empty() {
            "newest first"
        } else {
            "best match first"
        },
    );
    let frame = Frame {
        title: if q.is_empty() { "" } else { "Search" },
        description: "Typed wowlua libraries for World of Warcraft AddOns, by version.",
        tab: Tab::Packages,
        user: user.as_deref(),
    };
    render(&frame, &main)
}

// ---- a package ---------------------------------------------------------------

/// `GET /packages/<name>`.
pub async fn package(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    package_page(app, headers, name, None).await
}

/// `GET /packages/<name>/<version>`.
pub async fn package_version(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    package_page(app, headers, name, Some(version)).await
}

/// What a package's page reads.
struct Shown {
    entries: Vec<IndexEntry>,
    meta: Option<VersionMeta>,
    owners: Vec<String>,
    dependents: Vec<String>,
    /// Which dependencies are packages of this registry.
    known: Vec<String>,
    reserved: Option<Reserved>,
    /// Each version's publish date.
    dates: HashMap<String, u64>,
}

async fn package_page(
    app: Shared,
    headers: HeaderMap,
    name: String,
    version: Option<String>,
) -> Response {
    let user = viewer(&app, &headers);
    let path = match &version {
        Some(v) => format!("/packages/{name}/{v}"),
        None => format!("/packages/{name}"),
    };
    let valid = wowlua_pkg::registry::valid_name(&name);
    let shown = blocking({
        let app = app.clone();
        let name = name.clone();
        let version = version.clone();
        move || {
            let entries = if valid {
                app.store.entries(&name)
            } else {
                Vec::new()
            };
            let vers = match &version {
                Some(v) => entries
                    .iter()
                    .find(|e| e.vers == *v)
                    .map(|e| e.vers.clone()),
                None => newest(&entries).map(|e| e.vers.clone()),
            };
            let deps = vers
                .as_ref()
                .and_then(|v| entries.iter().find(|e| e.vers == *v))
                .map(|e| e.deps.clone())
                .unwrap_or_default();
            let dates = entries
                .iter()
                .map(|e| {
                    let published = app
                        .store
                        .version_meta(&name, &e.vers)
                        .map_or(0, |m| m.published);
                    (e.vers.clone(), published)
                })
                .collect();
            Shown {
                dates,
                meta: vers.and_then(|v| app.store.version_meta(&name, &v)),
                owners: app.store.owners(&name),
                dependents: app.store.dependents(&name),
                known: deps
                    .iter()
                    .filter(|d| d.registry.is_none() && !app.store.entries(&d.name).is_empty())
                    .map(|d| d.name.clone())
                    .collect(),
                reserved: if entries.is_empty() {
                    app.store.reserved().get(&name).cloned()
                } else {
                    None
                },
                entries,
            }
        }
    })
    .await;
    if shown.entries.is_empty() {
        let (help, action) = match &shown.reserved {
            Some(r) => (
                format!(
                    "`{name}` is reserved for the authors of {}; if you are one of them, claim it on GitHub",
                    if r.note.is_empty() {
                        name.as_str()
                    } else {
                        r.note.as_str()
                    }
                ),
                ("claim the name", claim_url(&name)),
            ),
            None => (
                format!("check the spelling, or search for it: wowlua search {name}"),
                ("search packages", format!("/?q={}", query_value(&name))),
            ),
        };
        return problem(
            &app,
            user.as_deref(),
            Problem {
                status: StatusCode::NOT_FOUND,
                message: format!("no package named `{name}` on this registry"),
                path: &path,
                help,
                action: (action.0, action.1),
            },
        );
    }
    let entries = &shown.entries;
    let selected = match &version {
        Some(v) => entries.iter().find(|e| e.vers == *v),
        None => newest(entries),
    };
    let (Some(entry), Some(meta)) = (selected, shown.meta.as_ref()) else {
        return problem(
            &app,
            user.as_deref(),
            Problem {
                status: StatusCode::NOT_FOUND,
                message: format!(
                    "`{name}` has no version {}",
                    version.as_deref().unwrap_or("")
                ),
                path: &path,
                help: format!("its versions are listed on its page: /packages/{name}"),
                action: ("all versions", format!("/packages/{name}#versions")),
            },
        );
    };
    let latest = newest(entries).map(|e| e.vers.clone()).unwrap_or_default();
    let mut versions: Vec<&IndexEntry> = entries.iter().collect();
    versions.sort_by_key(|e| std::cmp::Reverse(e.version()));
    let n = esc(&name);
    let req = requirement(&entry.vers);
    let wowlua = entry.wowlua.clone().unwrap_or_else(|| "any".into());
    let status = |e: &IndexEntry| {
        if e.yanked {
            ("yanked", "wl-badge wl-badge--warning")
        } else if e.vers == latest {
            ("latest", "wl-badge wl-badge--done")
        } else {
            ("", "")
        }
    };
    let options: String = versions
        .iter()
        .map(|e| {
            let (s, _) = status(e);
            format!(
                r#"<option value="/packages/{n}/{v}"{}>{v}{}</option>"#,
                if e.vers == entry.vers {
                    " selected"
                } else {
                    ""
                },
                if s.is_empty() {
                    String::new()
                } else {
                    format!(" ({s})")
                },
                v = esc(&e.vers),
            )
        })
        .collect();
    let keywords: String = meta
        .keywords
        .iter()
        .map(|k| format!(r#"<a href="/?q={}">{}</a>"#, query_value(k), esc(k)))
        .collect::<Vec<_>>()
        .join(" · ");
    let keywords = if keywords.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="caption rg-muted rg-facts-keywords">keywords: {keywords}</span>"#)
    };
    let repository = meta
        .repository
        .starts_with("https://")
        .then(|| meta.repository.clone());
    let rendered =
        (!meta.readme.is_empty()).then(|| readme::render(&meta.readme, &meta.repository));
    let mut links = vec![r##"<a href="#usage">usage</a>"##.to_string()];
    if rendered.is_some() {
        links.push(r##"<a href="#readme">readme</a>"##.into());
    }
    links.push(r##"<a href="#versions">versions</a>"##.into());
    links.push(r##"<a href="#dependencies">dependencies</a>"##.into());
    if let Some(r) = &repository {
        links.push(format!(r#"<a href="{}">repository ↗</a>"#, esc(r)));
    }
    let owners: String = shown
        .owners
        .iter()
        .map(|o| {
            format!(
                r#"<a href="https://github.com/{}">{} ↗</a>"#,
                esc(o),
                esc(o)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let license = if meta.license.is_empty() {
        "not given".to_string()
    } else {
        esc(&meta.license)
    };
    let usage_text = match meta.kind.as_str() {
        "types" => {
            "Add it to your package: its declarations type your calls into the code they describe."
        }
        "lua-library" => {
            "Add it to your package, then use it as its README says. wowlua builds the library into your AddOn, so players install nothing extra."
        }
        _ => {
            "Add it to your package, then import what you need. wowlua builds the library into your AddOn, so players install nothing extra."
        }
    };
    let manifest = format!("\"dependencies\": {{\n  \"{name}\": \"{req}\"\n}}");
    let figure = |file: &str, lang: &str, code: &str, class: &str| {
        format!(
            r#"<figure class="wl-code {class}" style="margin: 0;"><div class="wl-code__bar"><span class="wl-code__file">{file}</span><span class="wl-code__lang">{lang}</span><button class="wl-textbtn" type="button" aria-label="copy {file}">copy</button></div><pre class="wl-code__pre"><code>{}</code></pre></figure>"#,
            crate::highlight::highlight(code, lang)
        )
    };
    let example = rendered
        .as_ref()
        .and_then(|r| r.example.as_deref())
        .map(|code| figure("src/main.wowlua", "wowlua", code, ""))
        .unwrap_or_default();
    let readme_section = rendered
        .as_ref()
        .map(|r| {
            format!(
                r#"<section class="rg-section" aria-labelledby="readme"><h2 class="heading-2" id="readme">Readme</h2><div class="rg-readme body">{}</div></section>"#,
                r.html
            )
        })
        .unwrap_or_default();
    let rows: String = versions
        .iter()
        .map(|e| {
            let (s, class) = status(e);
            let published = shown_date(&shown.dates, e);
            let cksum = e.cksum.get(..15).unwrap_or(&e.cksum);
            format!(
                r#"<tr><td data-label="version"><a href="/packages/{n}/{v}"><code class="wl-icode">{v}</code></a></td><td data-label="requires">wowlua {w}</td><td data-label="published">{published}</td><td class="caption rg-muted" data-label="checksum" title="{full}">{cksum}…</td><td data-label="status"><span class="{class}">{s}</span></td></tr>"#,
                v = esc(&e.vers),
                w = esc(e.wowlua.as_deref().unwrap_or("any")),
                full = esc(&e.cksum),
                cksum = esc(cksum),
            )
        })
        .collect();
    let phone_rows: String = versions
        .iter()
        .map(|e| {
            let (s, class) = status(e);
            format!(
                r#"<li><a href="/packages/{n}/{v}"><code class="wl-icode">{v}</code></a><span class="{class}">{s}</span><span class="caption rg-muted">{} · wowlua {}</span></li>"#,
                shown_date(&shown.dates, e),
                esc(e.wowlua.as_deref().unwrap_or("any")),
                v = esc(&e.vers),
            )
        })
        .collect();
    let yanked_note = if entries.iter().any(|e| e.yanked) {
        r#"<p class="body-s rg-muted" style="margin: 0;">A yanked version stays in lockfiles that already have it; <code class="wl-icode">wowlua update</code> and new installs skip it, and <code class="wl-icode">wowlua check</code> warns W4322 where it is still locked.</p>"#
    } else {
        ""
    };
    let deps = if entry.deps.is_empty() {
        format!(
            r#"<p class="rg-empty body-s">{n} {} depends on the standard library only.</p>"#,
            esc(&entry.vers)
        )
    } else {
        let features = &entry.features;
        let rows: String = entry
            .deps
            .iter()
            .map(|d| {
                let when = if d.optional {
                    let by: Vec<&str> = features
                        .iter()
                        .filter(|(_, list)| {
                            list.iter().any(|x| {
                                x == &d.name
                                    || x.strip_prefix("dep:") == Some(d.name.as_str())
                                    || x.split_once('/').map(|(p, _)| p) == Some(d.name.as_str())
                            })
                        })
                        .map(|(f, _)| f.as_str())
                        .collect();
                    if by.is_empty() {
                        "optional".to_string()
                    } else {
                        format!("feature {}", by.join(", "))
                    }
                } else {
                    "always".to_string()
                };
                let package = if shown.known.contains(&d.name) {
                    format!(
                        r#"<a href="/packages/{0}"><code class="wl-icode">{0}</code></a>"#,
                        esc(&d.name)
                    )
                } else {
                    format!(
                        r#"<code class="wl-icode">{}</code>{}"#,
                        esc(&d.name),
                        d.registry
                            .as_deref()
                            .map(|r| format!(r#" <span class="caption rg-muted">{}</span>"#, esc(r)))
                            .unwrap_or_default()
                    )
                };
                format!(
                    r#"<tr><td data-label="package">{package}</td><td data-label="requirement"><code class="wl-icode">{}</code></td><td data-label="when">{}</td></tr>"#,
                    esc(&d.req),
                    esc(&when)
                )
            })
            .collect();
        format!(
            r#"<div class="wl-table" role="region" aria-label="Dependencies" tabindex="0"><table><thead><tr><th scope="col">package</th><th scope="col">requirement</th><th scope="col">when</th></tr></thead><tbody>{rows}</tbody></table></div>"#
        )
    };
    let used_by = if shown.dependents.is_empty() {
        format!("No package on this registry depends on {n} yet.")
    } else {
        shown
            .dependents
            .iter()
            .map(|d| format!(r#"<a href="/packages/{0}">{0}</a>"#, esc(d)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let report = match &repository {
        Some(r) => format!(
            r#"Open an issue in the <a href="{}">repository ↗</a>. A name or version that breaks the rules: <a href="https://wowlua.dev/community">tell us on Discord</a>."#,
            esc(r)
        ),
        None => r#"The package names no repository. A name or version that breaks the rules: <a href="https://wowlua.dev/community">tell us on Discord</a>."#.to_string(),
    };
    let main = format!(
        r##"<main class="rg-main">
<header class="wl-pkghead">
  <div>
    <p class="wl-pkghead__crumb"><a href="/">packages</a> / {n}</p>
    <h1 class="wl-pkghead__title">{n} <span class="wl-badge wl-badge--release">{kind}</span></h1>
    <p class="wl-pkghead__desc">{desc}</p>
    <div class="wl-badge-row" style="margin-bottom: var(--space-4);">{flavors}{keywords}</div>
    <nav class="wl-pkghead__links" aria-label="Package links">{links}</nav>
  </div>
  <div class="wl-pkghead__side">
    <div><label class="wl-pkghead__label" for="version">version</label><select class="wl-pkghead__select" id="version" data-navigate>{options}</select></div>
    <div><span class="wl-pkghead__label">install</span><div class="wl-pkghead__install"><code><span class="wl-term__prompt">$ </span>wowlua add <span class="rg-nowrap">{n}@{req}</span></code><button class="wl-textbtn" type="button" aria-label="copy install command" data-copy="wowlua add {n}@{req}">copy</button></div></div>
    <dl class="wl-pkghead__facts"><dt>requires</dt><dd>wowlua {wowlua}</dd><dt>license</dt><dd>{license}</dd><dt>published</dt><dd>{published}</dd><dt>owners</dt><dd>{owners}</dd></dl>
  </div>
</header>
<div class="rg-split" style="gap: 48px;">
  <div class="rg-split__main">
    <section class="rg-section" aria-labelledby="usage">
      <h2 class="heading-2" id="usage">Usage</h2>
      <p class="body" style="margin: 0; max-width: 72ch;">{usage_text}</p>
      {manifest}
      {example}
    </section>
    {readme_section}
    <section class="rg-section" aria-labelledby="versions">
      <div class="rg-row rg-row--start"><h2 class="heading-2" id="versions" style="margin: 0;">Versions</h2><span class="caption rg-muted">{count}</span></div>
      <div class="wl-table rg-versions-desk" role="region" aria-label="Versions" tabindex="0"><table><thead><tr><th scope="col">version</th><th scope="col">requires</th><th scope="col">published</th><th scope="col">checksum</th><th scope="col">status</th></tr></thead><tbody>{rows}</tbody></table></div>
      <ul class="rg-versions-phone">{phone_rows}</ul>
      {yanked_note}
    </section>
    <section class="rg-section" aria-labelledby="dependencies">
      <h2 class="heading-2" id="dependencies">Dependencies</h2>
      {deps}
    </section>
  </div>
  <aside class="rg-split__side" aria-label="About this package">
    <section class="rg-card"><h2 class="label"># used by</h2><p class="body-s">{used_by}</p></section>
    <section class="rg-card"><h2 class="label"># report a problem</h2><p class="body-s">{report}</p></section>
  </aside>
</div>
</main>"##,
        kind = esc(&meta.kind),
        desc = esc(&meta.description),
        flavors = badges(&meta.flavors),
        links = links.join(""),
        published = date(meta.published),
        manifest = figure("wowlua.json", "json", &manifest, "rg-usage-manifest"),
        count = if versions.len() == 1 {
            "1 version".to_string()
        } else {
            format!("{} versions", versions.len())
        },
    );
    let title = format!("{name} {}", entry.vers);
    let frame = Frame {
        title: &title,
        description: &meta.description,
        tab: Tab::None,
        user: user.as_deref(),
    };
    render(&frame, &main).into_response()
}

/// A version's publish date in the versions table.
fn shown_date(dates: &HashMap<String, u64>, e: &IndexEntry) -> String {
    dates.get(&e.vers).map(|d| date(*d)).unwrap_or_default()
}

// ---- sign-in -----------------------------------------------------------------

/// `GET /login`.
pub async fn login(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if viewer(&app, &headers).is_some() {
        return Redirect::to("/account").into_response();
    }
    let step = |no: u8, title: &str, text: &str, command: &str, label: &str| {
        format!(
            r#"<li><span class="heading-3"><span class="rg-step-no">{no}</span> {title}</span><p class="body-s rg-muted">{text}</p>{}</li>"#,
            copy_cmd(command, label)
        )
    };
    let main = format!(
        r#"<main class="rg-main rg-signin">
<section class="rg-signin__box" aria-labelledby="signin">
  <div style="display: grid; gap: 8px;">
    <h1 class="heading-1" id="signin">Sign in</h1>
    <p class="body rg-muted">Registry accounts are GitHub accounts. Sign in to create tokens for <code class="wl-icode">wowlua publish</code> and to manage the packages you own.</p>
  </div>
  <a class="wl-btn wl-btn--primary" href="/login/github">{github}sign in with GitHub</a>
  <dl class="body-s">
    <dt>asks for</dt><dd>your public profile only: no email, no repositories</dd>
    <dt>keeps</dt><dd>your GitHub login, as the owner name on your packages</dd>
    <dt>session</dt><dd>a cookie for this site; sign out from your account page</dd>
  </dl>
</section>
<section class="rg-steps" aria-labelledby="then">
  <h2 class="heading-2" id="then">Then, in your terminal</h2>
  <ol>{}{}{}</ol>
</section>
</main>"#,
        step(
            1,
            "Save the token",
            "Create one on your account page, then paste it into the hidden prompt. In CI, set <code class=\"wl-icode\">WOWLUA_REGISTRY_TOKEN</code> instead.",
            "wowlua login registry",
            "copy login command"
        ),
        step(
            2,
            "Look first",
            "See what the registry would get, without uploading.",
            "wowlua publish --dry-run",
            "copy dry run command"
        ),
        step(
            3,
            "Publish",
            "Runs your tests, packages the library and uploads it. A version, once published, never changes: fix it with a new one.",
            "wowlua publish",
            "copy publish command"
        ),
        github = include_str!("../assets/github.svg")
            .replace("<svg ", r#"<svg width="16" height="16" "#),
    );
    let frame = Frame {
        title: "Sign in",
        description: "Sign in to the wowlua registry with GitHub.",
        tab: Tab::None,
        user: None,
    };
    render(&frame, &main).into_response()
}

fn redirect_uri(app: &Shared) -> String {
    format!("{}/login/callback", app.config.public_url)
}

/// `GET /login/github`: off to GitHub, with a state to check on return.
pub async fn login_github(State(app): State<Shared>) -> Response {
    let state = auth::random_hex(16);
    let url = app.config.github.authorize_url(&redirect_uri(&app), &state);
    let mut response = Redirect::to(&url).into_response();
    if let Ok(v) = set_cookie(&app, STATE_COOKIE, &state, 600).parse() {
        response.headers_mut().insert(header::SET_COOKIE, v);
    }
    response
}

fn sign_in_failed(app: &Shared, text: &str, status: StatusCode) -> Response {
    problem(
        app,
        None,
        Problem {
            status,
            message: format!("sign-in failed: {text}"),
            path: "/login",
            help: "sign in with GitHub again".into(),
            action: ("sign in", "/login".into()),
        },
    )
}

/// `GET /login/callback?code&state`.
pub async fn callback(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let (Some(code), Some(state)) = (query.get("code"), query.get("state")) else {
        return sign_in_failed(
            &app,
            query
                .get("error_description")
                .map_or("GitHub sent no sign-in code", |s| s.as_str()),
            StatusCode::BAD_REQUEST,
        );
    };
    if cookie(&headers, STATE_COOKIE) != Some(state.as_str()) {
        return sign_in_failed(
            &app,
            "the sign-in expired or came from another page",
            StatusCode::BAD_REQUEST,
        );
    }
    let code = code.clone();
    let result = blocking({
        let app = app.clone();
        move || app.config.github.login(&code, &redirect_uri(&app))
    })
    .await;
    let login = match result {
        Ok(l) => l,
        Err(e) => return sign_in_failed(&app, &e, StatusCode::BAD_GATEWAY),
    };
    let id = app.sessions.start(&login);
    let mut response = Redirect::to("/account").into_response();
    let h = response.headers_mut();
    if let Ok(v) = set_cookie(&app, SESSION_COOKIE, &id, 7 * 24 * 60 * 60).parse() {
        h.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = set_cookie(&app, STATE_COOKIE, "", 0).parse() {
        h.append(header::SET_COOKIE, v);
    }
    response
}

/// The signed-in user, or the way to the sign-in page.
fn session(app: &Shared, headers: &HeaderMap) -> Result<(String, auth::Session), Response> {
    cookie(headers, SESSION_COOKIE)
        .and_then(|id| Some((id.to_string(), app.sessions.get(id)?)))
        .ok_or_else(|| Redirect::to("/login").into_response())
}

/// A form's session check: signed in and with the session's CSRF value.
fn form_session(
    app: &Shared,
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Result<auth::Session, Response> {
    let (_, s) = session(app, headers)?;
    if form.get("csrf") != Some(&s.csrf) {
        return Err(message(
            app,
            Some(&s.login),
            "the form expired; reload the page and send it again",
            StatusCode::FORBIDDEN,
        ));
    }
    Ok(s)
}

pub async fn logout(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Ok((id, s)) = session(&app, &headers)
        && form.get("csrf") == Some(&s.csrf)
    {
        app.sessions.end(&id);
    }
    let mut response = Redirect::to("/").into_response();
    if let Ok(v) = set_cookie(&app, SESSION_COOKIE, "", 0).parse() {
        response.headers_mut().insert(header::SET_COOKIE, v);
    }
    response
}

fn csrf_field(s: &auth::Session) -> String {
    format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        esc(&s.csrf)
    )
}

// ---- account -----------------------------------------------------------------

/// A package on the account page: its name, kind, owners and versions
/// (with each one's `wowlua` requirement and whether it is yanked).
struct Owned {
    name: String,
    kind: String,
    owners: Vec<String>,
    versions: Vec<IndexEntry>,
}

/// The account page; `new_token` shows a token just created, once.
async fn account_page(
    app: Shared,
    s: auth::Session,
    new_token: Option<(String, String)>,
) -> Response {
    let login = s.login.clone();
    let admin = app.config.is_admin(&login);
    let (tokens, packages, reserved) = blocking({
        let app = app.clone();
        let login = login.clone();
        move || {
            let tokens: Vec<Token> = app.store.tokens_of(&login);
            let names = if app.config.is_admin(&login) {
                app.store.packages()
            } else {
                app.store.owned_by(&login)
            };
            let packages: Vec<Owned> = names
                .into_iter()
                .map(|name| {
                    let mut versions = app.store.entries(&name);
                    versions.sort_by_key(|e| std::cmp::Reverse(e.version()));
                    let kind = newest(&versions)
                        .and_then(|e| app.store.version_meta(&name, &e.vers))
                        .map(|m| m.kind)
                        .unwrap_or_default();
                    Owned {
                        owners: app.store.owners(&name),
                        kind,
                        versions,
                        name,
                    }
                })
                .collect();
            (tokens, packages, app.store.reserved().len())
        }
    })
    .await;
    let csrf = csrf_field(&s);
    let callout = match &new_token {
        Some((name, token)) => format!(
            r#"<aside class="wl-callout wl-callout--tip rg-callout" aria-label="new token"><div class="wl-callout__head"><span class="wl-callout__glyph" aria-hidden="true">+</span>token created: {}</div><div class="wl-callout__body"><p>Copy it now. The registry keeps only its hash and can&#39;t show it again.</p><div class="wl-copy-cmd"><code class="wl-copy-cmd__text" data-new-token>{}</code><button class="wl-copy-cmd__btn" type="button" aria-label="copy token">copy</button></div><p>Then run <code class="wl-icode">wowlua login registry</code> and paste it into the hidden prompt, or set <code class="wl-icode">WOWLUA_REGISTRY_TOKEN</code> in CI.</p></div></aside>"#,
            esc(name),
            esc(token)
        ),
        None => String::new(),
    };
    let token_rows: String = tokens
        .iter()
        .map(|t| {
            format!(
                r#"<tr><td data-label="name">{name}</td><td class="caption rg-muted" data-label="id"><code>{id}</code></td><td data-label="created">{created}</td><td class="rg-actions"><form method="post" action="/account/tokens/revoke">{csrf}<input type="hidden" name="id" value="{id}"><button class="wl-btn wl-btn--danger wl-btn--small" aria-label="revoke token {name}">revoke</button></form></td></tr>"#,
                name = esc(&t.name),
                id = esc(&t.id),
                created = date(t.created),
            )
        })
        .collect();
    let token_table = if tokens.is_empty() {
        r#"<p class="rg-empty body-s">No tokens yet. Create one for each machine or CI that publishes.</p>"#.to_string()
    } else {
        format!(
            r#"<div class="wl-table" role="region" aria-label="API tokens" tabindex="0"><table><thead><tr><th scope="col">name</th><th scope="col">id</th><th scope="col">created</th><th scope="col"><span class="rg-hide">actions</span></th></tr></thead><tbody>{token_rows}</tbody></table></div>"#
        )
    };
    let steps: String = [
        ("1", "save the token", "wowlua login registry", "copy login command"),
        ("2", "look first", "wowlua publish --dry-run", "copy dry run command"),
        ("3", "publish", "wowlua publish", "copy publish command"),
    ]
    .iter()
    .map(|(no, title, cmd, label)| {
        format!(
            r#"<li><span class="label rg-muted"><span class="rg-step-no">{no}</span> · {title}</span>{}</li>"#,
            copy_cmd(cmd, label)
        )
    })
    .collect();
    let owned: String = packages
        .iter()
        .map(|p| {
            let name = esc(&p.name);
            let latest = newest(&p.versions).map(|e| e.vers.clone()).unwrap_or_default();
            let rows: String = p
                .versions
                .iter()
                .map(|e| {
                    let v = esc(&e.vers);
                    let (status, class) = if e.yanked {
                        ("yanked", "wl-badge wl-badge--warning")
                    } else if e.vers == latest {
                        ("latest", "wl-badge wl-badge--done")
                    } else {
                        ("", "")
                    };
                    let button = if e.yanked {
                        format!(r#"<button class="wl-btn wl-btn--secondary wl-btn--small">take back {v}</button>"#)
                    } else {
                        format!(r#"<button class="wl-btn wl-btn--danger wl-btn--small">yank {v}</button>"#)
                    };
                    format!(
                        r#"<tr><td data-label="version"><code class="wl-icode">{v}</code></td><td data-label="requires">wowlua {w}</td><td data-label="status"><span class="{class}">{status}</span></td><td class="rg-actions"><form method="post" action="/account/yank">{csrf}<input type="hidden" name="name" value="{name}"><input type="hidden" name="version" value="{v}"><input type="hidden" name="yanked" value="{flag}">{button}</form></td></tr>"#,
                        w = esc(e.wowlua.as_deref().unwrap_or("any")),
                        flag = if e.yanked { "0" } else { "1" },
                    )
                })
                .collect();
            let chips: String = p
                .owners
                .iter()
                .map(|o| {
                    let you = same_login(o, &login);
                    let end = if you {
                        r#"<span class="caption">you</span>"#.to_string()
                    } else {
                        format!(
                            r#"<form method="post" action="/account/owners">{csrf}<input type="hidden" name="name" value="{name}"><input type="hidden" name="remove" value="{o}"><button type="submit" aria-label="remove owner {o}">×</button></form>"#,
                            o = esc(o)
                        )
                    };
                    format!(r#"<span class="rg-owner body-s">@{}{end}</span>"#, esc(o))
                })
                .collect();
            let count = if p.versions.len() == 1 { "1 version".to_string() } else { format!("{} versions", p.versions.len()) };
            let badge = if p.kind.is_empty() {
                String::new()
            } else {
                format!(r#" <span class="wl-badge wl-badge--release rg-badge-gap">{}</span>"#, esc(&p.kind))
            };
            format!(
                r#"<article class="rg-panel">
  <div class="rg-owned__head"><h3 class="heading-3"><a href="/packages/{name}">{name}</a>{badge}</h3><span class="caption rg-muted">{count}</span></div>
  <div class="wl-table" role="region" aria-label="{name} versions" tabindex="0"><table><thead><tr><th scope="col">version</th><th scope="col">requires</th><th scope="col">status</th><th scope="col"><span class="rg-hide">actions</span></th></tr></thead><tbody>{rows}</tbody></table></div>
  <div class="rg-owners">
    <div><span class="label rg-muted">owners</span><div class="rg-owners__list">{chips}</div></div>
    <form class="rg-form" method="post" action="/account/owners">{csrf}<input type="hidden" name="name" value="{name}"><div class="wl-field"><label class="wl-field__label" for="owner-{name}">add an owner</label><div class="wl-input"><span class="wl-input__prefix" aria-hidden="true">@</span><input class="wl-input__control" id="owner-{name}" name="add" placeholder="GitHub login" maxlength="39" required></div></div><button class="wl-btn wl-btn--secondary">add</button></form>
  </div>
</article>"#
            )
        })
        .collect();
    let owned = if packages.is_empty() {
        r#"<p class="rg-empty body-s">You own no packages yet. <code class="wl-icode">wowlua publish</code> a library to create one.</p>"#.to_string()
    } else {
        owned
    };
    let admin_card = if admin {
        format!(
            r#"<section class="rg-admin" aria-labelledby="admin"><div><h2 class="heading-3" id="admin">Reserved names <span class="wl-badge wl-badge--dev rg-badge-gap">admin</span></h2><p class="body-s rg-muted">{reserved} names of well-known libraries (Ace3, LibStub, …) wait for their authors.</p></div><a class="wl-btn wl-btn--secondary" href="/account/reserved">manage reserved names</a></section>"#
        )
    } else {
        String::new()
    };
    let main = format!(
        r#"<main class="rg-main">
<header class="rg-account-head">
  <div><h1 class="display-l">Account</h1><p class="body rg-muted">Signed in with GitHub as <strong style="color: var(--ink);">{user}</strong>{admin_badge}</p></div>
  <form method="post" action="/logout">{csrf}<button class="wl-btn wl-btn--danger wl-btn--small">sign out</button></form>
</header>
{callout}
<section class="rg-section" aria-labelledby="tokens">
  <div class="rg-section__head"><h2 class="heading-2" id="tokens">API tokens</h2><p class="body-s rg-muted">One per machine or CI, so you can revoke one without the others. A token can publish, yank and change owners of your packages.</p></div>
  {token_table}
  <form class="rg-form" method="post" action="/account/tokens" style="max-width: 640px;">{csrf}<div class="wl-field"><label class="wl-field__label" for="tname">new token</label><div class="wl-input"><input class="wl-input__control" id="tname" name="name" maxlength="64" placeholder="laptop, ci, desktop" required></div></div><button class="wl-btn wl-btn--secondary">create token</button></form>
</section>
<section class="rg-panel rg-publish" aria-labelledby="publish">
  <div class="rg-publish__head"><div><h2 class="heading-2" id="publish">Publish a library</h2><p class="body-s rg-muted">With a token saved, publish from your package&#39;s folder. <code class="wl-icode">library</code>, <code class="wl-icode">lua-library</code> and <code class="wl-icode">types</code> packages go to the registry; AddOns go to CurseForge, Wago and WoWInterface.</p></div><a class="wl-btn wl-btn--ghost" href="https://wowlua.dev/docs/guide/publishing">read the guide</a></div>
  <ol>{steps}</ol>
</section>
<section class="rg-section" aria-labelledby="packages">
  <div class="rg-section__head"><h2 class="heading-2" id="packages">{packages_title}</h2><p class="body-s rg-muted">Yanking hides a version from new installs; lockfiles that have it keep it. You can take it back.</p></div>
  {owned}
</section>
{admin_card}
</main>"#,
        user = esc(&login),
        admin_badge = if admin {
            r#" <span class="wl-badge wl-badge--dev rg-badge-gap">admin</span>"#
        } else {
            ""
        },
        packages_title = if admin { "Packages" } else { "Your packages" },
    );
    let frame = Frame {
        title: "Account",
        description: "",
        tab: Tab::Account,
        user: Some(&login),
    };
    render(&frame, &main).into_response()
}

/// `GET /account`.
pub async fn account(State(app): State<Shared>, headers: HeaderMap) -> Response {
    match session(&app, &headers) {
        Ok((_, s)) => account_page(app, s, None).await,
        Err(r) => r,
    }
}

/// `POST /account/tokens`: a new token, shown once.
pub async fn token_create(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match form_session(&app, &headers, &form) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let name: String = form
        .get("name")
        .map(|n| n.trim().chars().take(64).collect())
        .filter(|n: &String| !n.is_empty())
        .unwrap_or_else(|| "token".into());
    let secret = auth::new_token();
    let hash = auth::hash_token(&secret);
    let token = Token {
        id: hash[..12].to_string(),
        hash,
        login: s.login.clone(),
        name: name.clone(),
        created: auth::now(),
    };
    let saved = blocking({
        let app = app.clone();
        move || app.store.add_token(token)
    })
    .await;
    if let Err(e) = saved {
        return message(
            &app,
            Some(&s.login),
            &e.to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        );
    }
    let mut response = account_page(app, s, Some((name, secret))).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("a header value"),
    );
    response
}

/// `POST /account/tokens/revoke`.
pub async fn token_revoke(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match form_session(&app, &headers, &form) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let id = form.get("id").cloned().unwrap_or_default();
    let login = s.login.clone();
    let _ = blocking(move || app.store.remove_token(&login, &id)).await;
    Redirect::to("/account").into_response()
}

fn api_failure(app: &Shared, login: &str, e: ApiError) -> Response {
    message(app, Some(login), &e.detail, e.status)
}

/// `POST /account/yank`: yank or unyank a version.
pub async fn yank(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match form_session(&app, &headers, &form) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let name = form.get("name").cloned().unwrap_or_default();
    let version = form.get("version").cloned().unwrap_or_default();
    let yanked = form.get("yanked").map(String::as_str) == Some("1");
    let owner = app
        .store
        .owners(&name)
        .iter()
        .any(|o| same_login(o, &s.login))
        || app.config.is_admin(&s.login);
    if !owner {
        return problem(
            &app,
            Some(&s.login),
            Problem {
                status: StatusCode::FORBIDDEN,
                message: format!("you are not an owner of `{name}`"),
                path: "/account",
                help: format!(
                    "an owner adds you on their account page, or with: wowlua owner add {name} <your GitHub login>"
                ),
                action: ("your account", "/account".into()),
            },
        );
    }
    let task = {
        let app = app.clone();
        move || app.store.set_yanked(&name, &version, yanked)
    };
    match blocking(task).await {
        Ok(true) => Redirect::to("/account").into_response(),
        Ok(false) => message(
            &app,
            Some(&s.login),
            "no such version",
            StatusCode::NOT_FOUND,
        ),
        Err(e) => message(
            &app,
            Some(&s.login),
            &e.to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    }
}

/// `POST /account/owners`: `add` or `remove` one login.
pub async fn owners(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match form_session(&app, &headers, &form) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let name = form.get("name").cloned().unwrap_or_default();
    let add = form
        .get("add")
        .map(|a| a.trim().trim_start_matches('@').to_string())
        .filter(|a| !a.is_empty());
    let remove = form.get("remove").cloned();
    let login = s.login.clone();
    let result = blocking({
        let app = app.clone();
        move || match (add, remove) {
            (Some(a), _) => crate::api::add_owners(&app, &name, &s.login, &[a]),
            (None, Some(r)) => crate::api::remove_owners(&app, &name, &s.login, &[r]),
            (None, None) => Ok(Vec::new()),
        }
    })
    .await;
    match result {
        Ok(_) => Redirect::to("/account").into_response(),
        Err(e) => api_failure(&app, &login, e),
    }
}

// ---- reserved names (admins) ---------------------------------------------------

fn not_admin(app: &Shared, login: &str) -> Response {
    problem(
        app,
        Some(login),
        Problem {
            status: StatusCode::FORBIDDEN,
            message: "only the registry's admins manage reserved names".into(),
            path: "/account/reserved",
            help: "to claim a reserved name, open the claim form on GitHub".into(),
            action: ("claim a name", claim_url("")),
        },
    )
}

/// `GET /account/reserved` (admins).
pub async fn reserved_page(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let s = match session(&app, &headers) {
        Ok((_, s)) => s,
        Err(r) => return r,
    };
    if !app.config.is_admin(&s.login) {
        return not_admin(&app, &s.login);
    }
    let all = blocking({
        let app = app.clone();
        move || app.store.reserved_all()
    })
    .await;
    let csrf = csrf_field(&s);
    let rows: String = all
        .iter()
        .map(|(name, r)| {
            let n = esc(name);
            let authors = r.authors.join(", ");
            let (status, class) = if r.released {
                ("released", "wl-badge wl-badge--planned")
            } else {
                ("reserved", "wl-badge")
            };
            let release = if r.released {
                String::new()
            } else {
                format!(
                    r#" <form method="post" action="/account/reserved">{csrf}<input type="hidden" name="name" value="{n}"><input type="hidden" name="release" value="1"><button class="wl-btn wl-btn--danger wl-btn--small" aria-label="release {n}">release</button></form>"#
                )
            };
            format!(
                r#"<tr><td data-label="name"><code class="wl-icode">{n}</code></td><td data-label="stands for">{note}</td><td class="body-s rg-muted" data-label="may publish">{shown}</td><td data-label="status"><span class="{class}">{status}</span></td><td class="rg-actions"><button class="wl-btn wl-btn--ghost wl-btn--small" type="button" data-edit="reserve-form" data-name="{n}" data-note="{note}" data-authors="{authors}" aria-label="edit {n}">edit</button>{release}</td></tr>"#,
                note = esc(&r.note),
                shown = if authors.is_empty() { "nobody yet".to_string() } else { esc(&authors) },
                authors = esc(&authors),
            )
        })
        .collect();
    let total = all.len();
    let main = format!(
        r#"<main class="rg-main" style="gap: 32px;">
<header class="rg-reserved-head">
  <p class="caption rg-muted"><a href="/account">account</a> / reserved names</p>
  <h1 class="heading-1">Reserved names <span class="wl-badge wl-badge--dev rg-badge-gap">admin</span></h1>
  <p class="body rg-muted">Only the listed GitHub users, and admins, may publish a reserved name first. An empty list keeps it for admins until they add its author. Releasing a name makes it first come, first served. Authors claim names with the <a href="{claim}">claim form on GitHub</a>.</p>
</header>
<div class="rg-filter-row">
  <div class="wl-field"><label class="wl-field__label" for="filter">filter</label><div class="wl-input"><span class="wl-input__prefix" aria-hidden="true">/</span><input class="wl-input__control" id="filter" type="search" data-filter-rows="reserved-table" data-filter-count="reserved-count"></div></div>
  <span class="caption rg-muted" id="reserved-count">{total} of {total} names</span>
</div>
<div class="wl-table" role="region" aria-label="Reserved names" tabindex="0"><table id="reserved-table"><thead><tr><th scope="col">name</th><th scope="col">stands for</th><th scope="col">may publish</th><th scope="col">status</th><th scope="col"><span class="rg-hide">actions</span></th></tr></thead><tbody>{rows}</tbody></table></div>
<section class="rg-panel rg-reserve" aria-labelledby="add">
  <h2 class="heading-2" id="add">Reserve a name</h2>
  <form id="reserve-form" method="post" action="/account/reserved">{csrf}
    <div class="rg-form">
      <div class="wl-field"><label class="wl-field__label" for="r-name">name</label><div class="wl-input"><input class="wl-input__control" id="r-name" name="name" placeholder="libfoo-1-0" aria-describedby="r-help" required></div></div>
      <div class="wl-field"><label class="wl-field__label" for="r-note">stands for</label><div class="wl-input"><input class="wl-input__control" id="r-note" name="note" placeholder="LibFoo-1.0"></div></div>
      <div class="wl-field"><label class="wl-field__label" for="r-authors">may publish</label><div class="wl-input"><input class="wl-input__control" id="r-authors" name="authors" placeholder="GitHub logins, comma separated" aria-describedby="r-help"></div></div>
      <button class="wl-btn wl-btn--secondary">reserve</button>
    </div>
    <p class="wl-field__help" id="r-help">Write the name as the registry spells it: lowercase, with - for . and spaces. Leave may publish empty to keep the name for admins until they add its author.</p>
  </form>
</section>
</main>"#,
        claim = esc(&claim_url("")),
    );
    let frame = Frame {
        title: "Reserved names",
        description: "",
        tab: Tab::Account,
        user: Some(&s.login),
    };
    render(&frame, &main).into_response()
}

/// `POST /account/reserved` (admins).
pub async fn reserved(
    State(app): State<Shared>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let s = match form_session(&app, &headers, &form) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !app.config.is_admin(&s.login) {
        return not_admin(&app, &s.login);
    }
    let name = form
        .get("name")
        .map(|n| n.trim().to_string())
        .unwrap_or_default();
    if !wowlua_pkg::registry::valid_name(&name) {
        return message(
            &app,
            Some(&s.login),
            &format!("`{name}` is not a package name"),
            StatusCode::BAD_REQUEST,
        );
    }
    let entry = if form.contains_key("release") {
        None
    } else {
        Some(Reserved {
            authors: form
                .get("authors")
                .map(|a| {
                    a.split(',')
                        .map(|x| x.trim().trim_start_matches('@').to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            note: form
                .get("note")
                .map(|n| n.trim().to_string())
                .unwrap_or_default(),
            released: false,
        })
    };
    let task = {
        let app = app.clone();
        move || app.store.set_reserved(&name, entry)
    };
    match blocking(task).await {
        Ok(()) => Redirect::to("/account/reserved").into_response(),
        Err(e) => message(
            &app,
            Some(&s.login),
            &e.to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    }
}

/// Any other path: the API's JSON answer for API paths and programs, a page
/// for browsers.
pub async fn not_found(State(app): State<Shared>, headers: HeaderMap, uri: Uri) -> Response {
    let path = uri.path().to_string();
    let browser = headers
        .get(header::ACCEPT)
        .and_then(|a| a.to_str().ok())
        .is_some_and(|a| a.contains("text/html"));
    let api = ["/v1/", "/index/", "/dl/"]
        .iter()
        .any(|p| path.starts_with(p));
    if api || !browser {
        return crate::api::not_found().await.into_response();
    }
    let user = viewer(&app, &headers);
    problem(
        &app,
        user.as_deref(),
        Problem {
            status: StatusCode::NOT_FOUND,
            message: "no such page on this registry".into(),
            path: &path,
            help: "the packages are on the registry's front page".into(),
            action: ("search packages", "/".into()),
        },
    )
}
