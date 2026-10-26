//! The registry's pages: a package list, sign-in with GitHub, and the
//! account page where a user creates and revokes API tokens, yanks
//! versions and manages the owners of their packages (and admins the
//! reserved names).

use crate::api::{ApiError, blocking};
use crate::store::{Reserved, Token, same_login};
use crate::{Shared, auth};
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use std::collections::HashMap;

const SESSION_COOKIE: &str = "wlr_session";
const STATE_COOKIE: &str = "wlr_state";

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

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · wowlua registry</title>
<style>
:root {{ --bg: #fbfaf7; --fg: #1d1b18; --muted: #6b665e; --line: #e3ded4; --accent: #b4561d; --card: #ffffff; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #16140f; --fg: #ece7dd; --muted: #a39d92; --line: #34302a; --accent: #e8894b; --card: #1e1b16; }} }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ max-width: 52rem; margin: 0 auto; padding: 2rem 1rem 4rem; }}
header {{ display: flex; justify-content: space-between; align-items: baseline; gap: 1rem; margin-bottom: 2rem; }}
header a {{ color: var(--fg); text-decoration: none; font-weight: 600; }}
h1 {{ font-size: 1.6rem; margin: 0 0 1rem; }}
h2 {{ font-size: 1.15rem; margin: 2rem 0 .75rem; }}
a {{ color: var(--accent); }}
p, li {{ color: var(--fg); }}
.muted {{ color: var(--muted); }}
section {{ background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 1rem 1.25rem; margin: 1rem 0; }}
table {{ width: 100%; border-collapse: collapse; }}
td, th {{ text-align: left; padding: .4rem .5rem .4rem 0; border-bottom: 1px solid var(--line); vertical-align: top; }}
code, .token {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .9em; }}
.token {{ display: block; padding: .75rem; background: var(--bg); border: 1px solid var(--line); border-radius: 6px; overflow-wrap: anywhere; user-select: all; }}
form.inline {{ display: inline; }}
input[type=text] {{ font: inherit; padding: .35rem .5rem; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); color: var(--fg); max-width: 100%; }}
button, .button {{ font: inherit; padding: .35rem .8rem; border: 1px solid var(--accent); border-radius: 6px; background: var(--accent); color: #fff; cursor: pointer; text-decoration: none; display: inline-block; }}
button.quiet {{ background: transparent; color: var(--accent); }}
.error {{ border-color: #c0392b; }}
</style>
</head>
<body><main>
<header><a href="/">wowlua registry</a><span><a href="/account">Account</a></span></header>
{body}
</main></body>
</html>"#,
        title = esc(title)
    ))
}

fn message(title: &str, text: &str, status: StatusCode) -> Response {
    (
        status,
        page(title, &format!("<h1>{}</h1><section class=\"error\"><p>{}</p></section><p><a href=\"/account\">Back</a></p>", esc(title), esc(text))),
    )
        .into_response()
}

/// `GET /`: what the registry holds.
pub async fn home(State(app): State<Shared>) -> Html<String> {
    let rows = blocking(move || {
        app.store
            .packages()
            .into_iter()
            .map(|name| {
                let entries = app.store.entries(&name);
                let latest = entries
                    .iter()
                    .filter(|e| !e.yanked)
                    .filter_map(|e| e.version())
                    .max()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "yanked".into());
                (name, latest)
            })
            .collect::<Vec<_>>()
    })
    .await;
    let mut body = String::from(
        "<h1>wowlua registry</h1><p class=\"muted\">Libraries for World of Warcraft addons built with <a href=\"https://wowlua.dev\">wowlua</a>. Depend on one with <code>\"name\": \"^1.0\"</code> in <code>wowlua.json</code>; publish yours with <code>wowlua publish</code> after <a href=\"/login\">signing in</a> for a token.</p>",
    );
    if rows.is_empty() {
        body.push_str("<section><p class=\"muted\">No packages yet.</p></section>");
    } else {
        body.push_str("<section><table><tr><th>Package</th><th>Latest</th></tr>");
        for (name, latest) in rows {
            body.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{}</td></tr>",
                esc(&name),
                esc(&latest)
            ));
        }
        body.push_str("</table></section>");
    }
    page("Packages", &body)
}

/// `GET /login`.
pub async fn login(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if cookie(&headers, SESSION_COOKIE).is_some_and(|id| app.sessions.get(id).is_some()) {
        return Redirect::to("/account").into_response();
    }
    page(
        "Sign in",
        "<h1>Sign in</h1><section><p>The registry's accounts are GitHub accounts: sign in with GitHub to create API tokens for <code>wowlua publish</code>, and to manage the packages you own.</p><p><a class=\"button\" href=\"/login/github\">Sign in with GitHub</a></p><p class=\"muted\">The registry reads your public GitHub login only.</p></section>",
    )
    .into_response()
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

/// `GET /login/callback?code&state`.
pub async fn callback(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let (Some(code), Some(state)) = (query.get("code"), query.get("state")) else {
        return message(
            "Sign-in failed",
            query
                .get("error_description")
                .map_or("GitHub sent no sign-in code.", |s| s.as_str()),
            StatusCode::BAD_REQUEST,
        );
    };
    if cookie(&headers, STATE_COOKIE) != Some(state.as_str()) {
        return message(
            "Sign-in failed",
            "The sign-in expired or came from another page; sign in again.",
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
        Err(e) => return message("Sign-in failed", &e, StatusCode::BAD_GATEWAY),
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
            "Not sent",
            "The form expired; reload the page and try again.",
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

/// A package on the account page: its name, owners and versions (with
/// whether each is yanked).
type Owned = (String, Vec<String>, Vec<(String, bool)>);

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
                .map(|n| {
                    let owners = app.store.owners(&n);
                    let versions = app
                        .store
                        .entries(&n)
                        .into_iter()
                        .map(|e| (e.vers, e.yanked))
                        .collect();
                    (n, owners, versions)
                })
                .collect();
            (tokens, packages, app.store.reserved())
        }
    })
    .await;
    let csrf = csrf_field(&s);
    let mut body = format!(
        "<h1>{}</h1><p class=\"muted\">Signed in with GitHub as <strong>{}</strong>{}.</p><form class=\"inline\" method=\"post\" action=\"/logout\">{csrf}<button class=\"quiet\">Sign out</button></form>",
        esc(&login),
        esc(&login),
        if admin { " (admin)" } else { "" }
    );
    if let Some((name, token)) = new_token {
        body.push_str(&format!(
            "<section><h2>New token: {}</h2><p>Copy it now; the registry keeps only its hash and cannot show it again.</p><code class=\"token\">{}</code><p class=\"muted\">Save it with <code>wowlua login registry</code>, or set <code>WOWLUA_REGISTRY_TOKEN</code> in CI.</p></section>",
            esc(&name),
            esc(&token)
        ));
    }
    body.push_str("<h2>API tokens</h2><section>");
    if tokens.is_empty() {
        body.push_str("<p class=\"muted\">No tokens yet.</p>");
    } else {
        body.push_str("<table><tr><th>Name</th><th>Created</th><th></th></tr>");
        for t in &tokens {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td><form class=\"inline\" method=\"post\" action=\"/account/tokens/revoke\">{csrf}<input type=\"hidden\" name=\"id\" value=\"{}\"><button class=\"quiet\">Revoke</button></form></td></tr>",
                esc(&t.name),
                date(t.created),
                esc(&t.id)
            ));
        }
        body.push_str("</table>");
    }
    body.push_str(&format!(
        "<form method=\"post\" action=\"/account/tokens\">{csrf}<p><input type=\"text\" name=\"name\" placeholder=\"Token name, e.g. laptop or CI\" maxlength=\"64\" required> <button>Create token</button></p></form></section>"
    ));
    body.push_str("<h2>Packages</h2>");
    if packages.is_empty() {
        body.push_str("<section><p class=\"muted\">You own no packages yet. <code>wowlua publish</code> a library to create one.</p></section>");
    }
    for (name, owners, versions) in &packages {
        body.push_str(&format!(
            "<section><h2><code>{}</code></h2><table><tr><th>Version</th><th></th></tr>",
            esc(name)
        ));
        for (v, yanked) in versions.iter().rev() {
            body.push_str(&format!(
                "<tr><td>{}{}</td><td><form class=\"inline\" method=\"post\" action=\"/account/yank\">{csrf}<input type=\"hidden\" name=\"name\" value=\"{}\"><input type=\"hidden\" name=\"version\" value=\"{}\"><input type=\"hidden\" name=\"yanked\" value=\"{}\"><button class=\"quiet\">{}</button></form></td></tr>",
                esc(v),
                if *yanked { " <span class=\"muted\">(yanked)</span>" } else { "" },
                esc(name),
                esc(v),
                if *yanked { "0" } else { "1" },
                if *yanked { "Unyank" } else { "Yank" }
            ));
        }
        body.push_str("</table><p>Owners: ");
        for (i, o) in owners.iter().enumerate() {
            if i > 0 {
                body.push_str(", ");
            }
            body.push_str(&format!(
                "{} <form class=\"inline\" method=\"post\" action=\"/account/owners\">{csrf}<input type=\"hidden\" name=\"name\" value=\"{}\"><input type=\"hidden\" name=\"remove\" value=\"{}\"><button class=\"quiet\" title=\"Remove\">×</button></form>",
                esc(o),
                esc(name),
                esc(o)
            ));
        }
        body.push_str(&format!(
            "</p><form method=\"post\" action=\"/account/owners\">{csrf}<input type=\"hidden\" name=\"name\" value=\"{}\"><input type=\"text\" name=\"add\" placeholder=\"GitHub login\" maxlength=\"39\" required> <button>Add owner</button></form></section>",
            esc(name)
        ));
    }
    if admin {
        body.push_str("<h2>Reserved names</h2><section><p class=\"muted\">Only the listed GitHub users (and admins) may publish a reserved name first. An empty list keeps it for the admins.</p><table><tr><th>Name</th><th>For</th><th>Authors</th></tr>");
        for (name, r) in &reserved {
            body.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                esc(name),
                esc(&r.note),
                esc(&r.authors.join(", "))
            ));
        }
        body.push_str(&format!(
            "</table><form method=\"post\" action=\"/account/reserved\">{csrf}<p><input type=\"text\" name=\"name\" placeholder=\"name\" required> <input type=\"text\" name=\"authors\" placeholder=\"GitHub logins, comma-separated\"> <input type=\"text\" name=\"note\" placeholder=\"what it is\"> <button>Reserve or assign</button> <button class=\"quiet\" name=\"release\" value=\"1\">Release</button></p></form></section>"
        ));
    }
    page("Account", &body).into_response()
}

/// `YYYY-MM-DD` of a time in seconds.
fn date(secs: u64) -> String {
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
            "Not saved",
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

fn api_failure(e: ApiError) -> Response {
    message("Not changed", &e.detail, e.status)
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
        return message(
            "Not changed",
            "You do not own this package.",
            StatusCode::FORBIDDEN,
        );
    }
    match blocking(move || app.store.set_yanked(&name, &version, yanked)).await {
        Ok(true) => Redirect::to("/account").into_response(),
        Ok(false) => message("Not changed", "No such version.", StatusCode::NOT_FOUND),
        Err(e) => message(
            "Not changed",
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
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let remove = form.get("remove").cloned();
    let result = blocking(move || match (add, remove) {
        (Some(a), _) => crate::api::add_owners(&app, &name, &s.login, &[a]),
        (None, Some(r)) => crate::api::remove_owners(&app, &name, &s.login, &[r]),
        (None, None) => Ok(Vec::new()),
    })
    .await;
    match result {
        Ok(_) => Redirect::to("/account").into_response(),
        Err(e) => api_failure(e),
    }
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
        return message(
            "Not changed",
            "Only admins manage reserved names.",
            StatusCode::FORBIDDEN,
        );
    }
    let name = form
        .get("name")
        .map(|n| n.trim().to_string())
        .unwrap_or_default();
    if !wowlua_pkg::registry::valid_name(&name) {
        return message(
            "Not changed",
            "That is not a package name.",
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
                        .map(|x| x.trim().to_string())
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
    match blocking(move || app.store.set_reserved(&name, entry)).await {
        Ok(()) => Redirect::to("/account").into_response(),
        Err(e) => message(
            "Not changed",
            &e.to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    }
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
    }
}
