//! Files the BigWigs packager puts into a package besides the addon's own
//! (decision T146): the changelog it generates when a project has no manual
//! one, and the license `license-output` names when the repository has
//! none (downloaded from the project's wowace.com page).

use crate::keywords::KeywordInfo;
use std::path::Path;
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A title without color codes and textures, as the packager uses the
/// TOC's `## Title` for the changelog.
pub fn plain_title(title: &str) -> String {
    let mut out = String::new();
    let mut rest = title;
    while let Some(i) = rest.find('|') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let is_hex = |s: &str| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if tail.len() >= 10 && tail.as_bytes()[1] == b'c' && is_hex(&tail[2..10]) {
            rest = &tail[10..];
        } else if let Some(after) = tail.strip_prefix("|r") {
            rest = after;
        } else if tail.starts_with("|T")
            && let Some(end) = tail[2..].find("|t")
        {
            rest = &tail[2 + end + 2..];
        } else {
            out.push('|');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// The repository's GitHub URL, as the packager reads it from `origin`.
fn github_url(dir: &Path) -> Option<String> {
    let remote = git(dir, &["remote", "get-url", "origin"])
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            git(dir, &["remote", "-v"])?
                .lines()
                .find(|l| l.ends_with("(fetch)"))
                .and_then(|l| l.split_whitespace().nth(1))
                .map(str::to_string)
        })?;
    let remote = remote.trim();
    let url = match remote.strip_prefix("git@").and_then(|r| r.split_once(':')) {
        Some((host, path)) => format!("https://{host}/{path}"),
        None => remote.to_string(),
    };
    url.starts_with("https://github.com")
        .then(|| url.trim_end_matches(".git").to_string())
}

/// The packager's `sed` pass over `git log --pretty=format:"###%B"`.
fn format_log(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.split('\n') {
        let mut line = format!("    {line}");
        if line.trim_matches(' ').is_empty() {
            line.clear();
        }
        if let Some(rest) = line.strip_prefix("    ###") {
            line = format!("- {rest}");
        }
        line.push_str("  ");
        line = escape_underscores(&line).replace("###", "_");
        line = line.replace("[ci skip]", "").replace("[skip ci]", "");
        if line.contains("git-svn-id:") || is_revert_note(&line) || line.trim().is_empty() {
            continue;
        }
        out.push(line);
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// `_` outside backtick spans becomes `\###` (later `\_`), up to an
/// unclosed backtick.
fn escape_underscores(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let Some(i) = rest.find(['`', '_']) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..i]);
        if rest.as_bytes()[i] == b'_' {
            out.push_str("\\###");
            rest = &rest[i + 1..];
            continue;
        }
        match rest[i + 1..].find('`') {
            Some(j) => {
                out.push_str(&rest[i..i + 1 + j + 1]);
                rest = &rest[i + 1 + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                break;
            }
        }
    }
    out
}

fn is_revert_note(line: &str) -> bool {
    let t = line.trim();
    t.strip_prefix("This reverts commit ")
        .and_then(|r| r.strip_suffix('.'))
        .is_some_and(|h| h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The changelog the packager generates (`CHANGELOG.md`): the commits since
/// the previous release tag, with GitHub links when the repository is on
/// GitHub. `None` outside a git repository.
pub fn generated_changelog(dir: &Path, title: &str, info: &KeywordInfo) -> Option<String> {
    git(dir, &["rev-parse", "HEAD"])?;
    let exclude_alpha = "--exclude=*[Aa][Ll][Pp][Hh][Aa]*";
    let tag = crate::keywords::head_tag(dir);
    let line = |s: Option<String>| s.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let previous = match &tag {
        Some(t) => {
            let lower = t.to_ascii_lowercase();
            let mut args = vec!["describe", "--tags", "--abbrev=0", exclude_alpha];
            if !lower.contains("beta") && !lower.contains("alpha") {
                args.push("--exclude=*[Bb][Ee][Tt][Aa]*");
            }
            args.push("HEAD~");
            line(git(dir, &args))
        }
        None => line(git(
            dir,
            &["describe", "--tags", "--abbrev=7", exclude_alpha],
        ))
        .and_then(|_| {
            line(git(
                dir,
                &["describe", "--tags", "--abbrev=0", exclude_alpha],
            ))
        }),
    };
    let url = github_url(dir);
    let hash = &info.hash;
    let version = &info.version;
    let (range, link, version_link) = match (&previous, &tag) {
        (None, None) => (
            hash.clone(),
            format!("commits/{hash}"),
            format!("tree/{hash}"),
        ),
        (None, Some(t)) => (t.clone(), format!("commits/{t}"), format!("tree/{t}")),
        (Some(p), None) => (
            format!("{p}..{hash}"),
            format!("compare/{p}...{hash}"),
            format!("tree/{hash}"),
        ),
        (Some(p), Some(t)) => (
            format!("{p}..{t}"),
            format!("compare/{p}...{t}"),
            format!("tree/{t}"),
        ),
    };
    // Links to previous releases only when the packager makes releases.
    let token = ["GH_API_TOKEN", "GITHUB_API_TOKEN", "GITHUB_OAUTH"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    let (changelog_url, changelog_version, changelog_previous) = match &url {
        Some(u) => (
            format!("[Full Changelog]({u}/{link})"),
            format!("[{version}]({u}/{version_link})"),
            if token {
                format!("[Previous Releases]({u}/releases)")
            } else {
                String::new()
            },
        ),
        None => (String::new(), version.clone(), String::new()),
    };
    let date = &crate::keywords::iso_date(info.timestamp)[..10];
    let log = git(dir, &["log", &range, "--pretty=format:###%B"]).unwrap_or_default();
    Some(format!(
        "# {title}\n\n## {changelog_version} ({date})\n{changelog_url} {changelog_previous}\n\n{}",
        format_log(&log)
    ))
}

/// The license text of a CurseForge project's wowace.com page, as the
/// packager saves it for `license-output`; cached, `None` when unavailable.
pub fn downloaded_license(project_id: &str) -> Option<Vec<u8>> {
    if !project_id.bytes().all(|b| b.is_ascii_digit()) || project_id.is_empty() {
        return None;
    }
    let cache = wowlua_pkg::Cache::new()?;
    let entry = cache.checkout_dir(&format!("license-{project_id}"));
    let file = entry.join("LICENSE");
    if let Ok(bytes) = std::fs::read(&file) {
        return Some(bytes);
    }
    let url = format!("https://www.wowace.com/project/{project_id}/license");
    let page = ureq::get(&url)
        .call()
        .ok()?
        .into_body()
        .read_to_string()
        .ok()?;
    // `sed -e '1,4d' -e '5s/^\s*//' -e '$d' | sed '$d'`: the text without
    // the page's wrapping lines.
    let mut lines: Vec<String> = page.lines().skip(4).map(str::to_string).collect();
    if let Some(first) = lines.first_mut() {
        *first = first.trim_start().to_string();
    }
    lines.pop();
    lines.pop();
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::create_dir_all(&entry).ok()?;
    std::fs::write(&file, &text).ok()?;
    Some(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_commit_messages_like_the_packager() {
        let raw = "###Fix my_var in `a_b` [ci skip]\n\nbody line\n###Revert x\n\nThis reverts commit 0123456789012345678901234567890123456789.\n";
        assert_eq!(
            format_log(raw),
            "- Fix my\\_var in `a_b`   \n    body line  \n- Revert x  \n"
        );
        assert_eq!(
            plain_title("|cff9966ffKui|r Nameplates |Tx:0|t"),
            "Kui Nameplates"
        );
    }
}
