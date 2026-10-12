//! Changelogs (Publishing §11.2.1): the file's section for the version, a
//! git-generated list, or none; converted packager projects send what the
//! packager sends (its manual or generated changelog, and for WoWInterface
//! its BBCode).

use std::path::Path;
use wowlua_manifest::model::ChangelogFormat;

/// A changelog and its format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changelog {
    pub text: String,
    pub format: ChangelogFormat,
}

impl Changelog {
    /// CurseForge's `changelogType`.
    pub fn curseforge_type(&self) -> &'static str {
        match self.format {
            ChangelogFormat::Markdown => "markdown",
            ChangelogFormat::Html => "html",
            ChangelogFormat::Text => "text",
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The section of a changelog file for `version`, when the file has
/// version headings (`## 1.4.2`, `## [1.4.2]`, `# v1.4.2`); otherwise the
/// whole file.
pub fn section(text: &str, version: &str) -> String {
    let plain = version.trim_start_matches('v');
    let heading = |line: &str| -> Option<(usize, bool)> {
        let level = line.chars().take_while(|c| *c == '#').count();
        if level == 0 || !line[level..].starts_with(' ') {
            return None;
        }
        let words: Vec<String> = line[level..]
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | ':'))
                    .trim_start_matches('v')
                    .to_string()
            })
            .collect();
        let versioned = words.iter().any(|w| {
            w.split('.').count() >= 2
                && w.split('.')
                    .all(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        });
        versioned.then(|| (level, words.iter().any(|w| w == plain)))
    };
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines
        .iter()
        .position(|l| heading(l).is_some_and(|(_, m)| m))
    else {
        return text.to_string();
    };
    let (level, _) = heading(lines[start]).unwrap_or((2, true));
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            let n = l.chars().take_while(|c| *c == '#').count();
            n > 0 && n <= level && l[n..].starts_with(' ')
        })
        .map_or(lines.len(), |i| start + 1 + i);
    let mut out = lines[start..end].join("\n");
    out = out.trim_end().to_string();
    out.push('\n');
    out
}

/// A changelog from the commits since the previous tag: a heading and one
/// item per commit subject with its short hash, merges left out.
pub fn from_git(repo: &Path, name: &str, version: &str) -> String {
    let previous = git(repo, &["describe", "--tags", "--abbrev=0", "HEAD~"])
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let range = match &previous {
        Some(p) => format!("{p}..HEAD"),
        None => "HEAD".into(),
    };
    let log = git(repo, &["log", &range, "--no-merges", "--format=%s (%h)"]).unwrap_or_default();
    let mut out = format!("## {name} {version}\n\n");
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(&format!("- {line}\n"));
    }
    out
}

/// Markdown → BBCode for WoWInterface: headings, list items, links, code,
/// emphasis.
pub fn bbcode(markdown: &str) -> String {
    let inline = |line: &str| -> String {
        let link = regex::Regex::new(r"\[([^\]]+)\]\(([^)\s]+)\)").expect("a regex");
        let code = regex::Regex::new(r"`([^`]+)`").expect("a regex");
        let bold = regex::Regex::new(r"\*\*([^*]+)\*\*|__([^_]+)__").expect("a regex");
        let italic = regex::Regex::new(r"\*([^*\s][^*]*)\*|\b_([^_]+)_\b").expect("a regex");
        let s = link.replace_all(line, "[url=$2]$1[/url]").into_owned();
        let s = code
            .replace_all(&s, "[font=monospace]$1[/font]")
            .into_owned();
        let s = bold
            .replace_all(&s, |c: &regex::Captures| {
                format!(
                    "[b]{}[/b]",
                    c.get(1).or(c.get(2)).map_or("", |m| m.as_str())
                )
            })
            .into_owned();
        italic
            .replace_all(&s, |c: &regex::Captures| {
                format!(
                    "[i]{}[/i]",
                    c.get(1).or(c.get(2)).map_or("", |m| m.as_str())
                )
            })
            .into_owned()
    };
    let mut out = String::new();
    let mut in_list = false;
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_list {
                out.push_str("[/list]\n");
                in_list = false;
            }
            out.push_str(if in_code { "[/code]\n" } else { "[code]" });
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let item = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "));
        if let Some(item) = item {
            if !in_list {
                out.push_str("[list]\n");
                in_list = true;
            }
            out.push_str(&format!("[*]{}\n", inline(item)));
            continue;
        }
        if in_list {
            out.push_str("[/list]\n");
            in_list = false;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level > 0 && trimmed[level..].starts_with(' ') {
            let size = match level {
                1 => 6,
                2 => 5,
                3 => 4,
                _ => 3,
            };
            out.push_str(&format!(
                "[size={size}][b]{}[/b][/size]\n",
                inline(trimmed[level..].trim())
            ));
            continue;
        }
        out.push_str(&inline(line));
        out.push('\n');
    }
    if in_list {
        out.push_str("[/list]\n");
    }
    if in_code {
        out.push_str("[/code]\n");
    }
    out
}

/// HTML → Markdown for Wago: headings, list items, links, code, emphasis
/// and paragraphs; other tags are dropped.
pub fn markdown_of_html(html: &str) -> String {
    let rules: &[(&str, &str)] = &[
        (r"(?i)<h([1-6])[^>]*>", "\n<h$1>"),
        (r"(?i)</h[1-6]>", "\n"),
        (r"(?i)<li[^>]*>", "\n- "),
        (r"(?i)</(ul|ol|p|div)>|<br\s*/?>", "\n"),
        (r#"(?i)<a [^>]*href="([^"]*)"[^>]*>(.*?)</a>"#, "[$2]($1)"),
        (r"(?i)</?(strong|b)>", "**"),
        (r"(?i)</?(em|i)>", "*"),
        (r"(?i)</?code>", "`"),
    ];
    let mut s = html.to_string();
    for (pattern, replacement) in rules {
        s = regex::Regex::new(pattern)
            .expect("a regex")
            .replace_all(&s, *replacement)
            .into_owned();
    }
    let heading = regex::Regex::new(r"<h([1-6])>").expect("a regex");
    s = heading
        .replace_all(&s, |c: &regex::Captures| {
            format!("{} ", "#".repeat(c[1].parse().unwrap_or(1)))
        })
        .into_owned();
    s = regex::Regex::new(r"<[^>]+>")
        .expect("a regex")
        .replace_all(&s, "")
        .into_owned();
    let s = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut out = String::new();
    let mut blank = true;
    for line in s.lines().map(str::trim) {
        if line.is_empty() {
            if !blank {
                out.push('\n');
            }
            blank = true;
        } else {
            out.push_str(line);
            out.push('\n');
            blank = false;
        }
    }
    out
}

/// The packager's Markdown → BBCode for WoWInterface: `pandoc` to HTML,
/// then its `sed` rules. `None` without `pandoc`.
pub fn packager_bbcode(markdown_file: &Path) -> Option<String> {
    let out = std::process::Command::new("pandoc")
        .args(["--wrap=none", "-f", "commonmark", "-t", "html"])
        .arg(markdown_file)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let html = String::from_utf8_lossy(&out.stdout).into_owned();
    let rules: &[(&str, &str)] = &[
        (r"<(/)?(b|i|u)>", "[$1$2]"),
        (r"<(/)?em>", "[${1}i]"),
        (r"<(/)?strong>", "[${1}b]"),
        (r"<ul[^>]*>", "[list]"),
        (r"<ol[^>]*>", "[list=\"1\"]"),
        (r"</[ou]l>", "[/list]\n"),
        (r"<li><p>", "[*]"),
        (r"<li>", "[*]"),
        (r"</p></li>", ""),
        (r"</li>", ""),
        (r"\[\*\]\[ \] ", "[*]☐ "),
        (r"\[\*\]\[[xX]\] ", "[*]☒ "),
        (r"<h1[^>]*>", "[size=\"6\"]"),
        (r"<h2[^>]*>", "[size=\"5\"]"),
        (r"<h3[^>]*>", "[size=\"4\"]"),
        (r"<h[456][^>]*>", "[size=\"3\"]"),
        (r"</h[1-6]>", "[/size]\n"),
        (r"<blockquote>", "[quote]"),
        (r"</blockquote>", "[/quote]\n"),
        (
            r#"<div class="sourceCode"[^>]*><pre class="sourceCode lua"><code class="sourceCode lua">"#,
            "[highlight=\"lua\"]",
        ),
        (r"</code></pre></div>", "[/highlight]\n"),
        (r"<pre><code>", "[code]"),
        (r"</code></pre>", "[/code]\n"),
        (r"<code>", "[font=\"monospace\"]"),
        (r"</code>", "[/font]"),
        (r#"<a href="([^"]+)"[^>]*>"#, "[url=\"$1\"]"),
        (r"</a>", "[/url]"),
        (r#"<img src="([^"]+)"[^>]*>"#, "[img]$1[/img]"),
        (
            r"<hr />",
            "_____________________________________________________________________________\n",
        ),
        (r"</p>", "\n"),
    ];
    let mut lines: Vec<String> = Vec::new();
    for line in html.lines() {
        let mut s = line.to_string();
        for (pattern, replacement) in rules {
            let re = regex::Regex::new(pattern).expect("a regex");
            s = re.replace_all(&s, *replacement).into_owned();
        }
        // `sed` works per line: replacements with `\n` add lines.
        for part in s.split('\n') {
            lines.push(part.to_string());
        }
    }
    let tag_only = regex::Regex::new(r"^<[^>]+>$").expect("a regex");
    let any_tag = regex::Regex::new(r"<[^>]+>").expect("a regex");
    let text: Vec<String> = lines
        .into_iter()
        .filter(|l| !tag_only.is_match(l))
        .map(|l| {
            any_tag
                .replace_all(&l, "")
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&#39;", "'")
        })
        .collect();
    Some(text.join("\n"))
}

/// The packager's generated WoWInterface changelog (BBCode), for tagged
/// builds: title, version and date, links, and the commits as a list.
#[allow(clippy::too_many_arguments)]
pub fn packager_wowi_generated(
    repo: &Path,
    title: &str,
    version: &str,
    timestamp: i64,
    previous: Option<&str>,
    tag: &str,
    github: Option<&str>,
    token: bool,
) -> String {
    let date = &crate::keywords::iso_date(timestamp)[..10];
    let (range, link) = match previous {
        Some(p) => (format!("{p}..{tag}"), format!("compare/{p}...{tag}")),
        None => (tag.to_string(), format!("commits/{tag}")),
    };
    let (version_line, url_line, previous_line) = match github {
        Some(u) => (
            format!("[url={u}/tree/{tag}]{version}[/url]"),
            format!("[url={u}/{link}]Full Changelog[/url]"),
            if token {
                format!("[url={u}/releases]Previous Releases[/url]")
            } else {
                String::new()
            },
        ),
        None => (
            format!("[color=orange]{version}[/color]"),
            String::new(),
            String::new(),
        ),
    };
    let mut out = format!(
        "[size=5]{title}[/size]\n[size=4]{version_line} ({date})[/size]\n{url_line} {previous_line}\n[list]\n"
    );
    let log = git(repo, &["log", &range, "--pretty=format:###%B"]).unwrap_or_default();
    for line in log.split('\n') {
        let mut s = format!("    {line}");
        if s.trim_matches(' ').is_empty() {
            s.clear();
        }
        if let Some(rest) = s.strip_prefix("    ###") {
            s = format!("[*]{rest}");
        }
        s = s.replace("[ci skip]", "").replace("[skip ci]", "");
        if s.contains("git-svn-id:") || crate::packaging::is_revert_note(&s) || s.trim().is_empty()
        {
            continue;
        }
        out.push_str(&s);
        out.push('\n');
    }
    out.push_str("[/list]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_html_to_markdown() {
        let html = "<h2>1.4.2</h2><ul><li><b>Fixed</b> <a href=\"https://x.y\">a bug</a></li></ul><p>A &amp; B</p>";
        assert_eq!(
            markdown_of_html(html),
            "## 1.4.2\n\n- **Fixed** [a bug](https://x.y)\nA & B\n"
        );
    }

    #[test]
    fn takes_the_version_section() {
        let text = "# Changelog\n\n## [1.4.2] - 2026-10-01\n- fix\n\n## 1.4.1\n- old\n";
        assert_eq!(section(text, "v1.4.2"), "## [1.4.2] - 2026-10-01\n- fix\n");
        assert_eq!(section(text, "2.0.0"), text);
        assert_eq!(section("no headings\n", "1.0.0"), "no headings\n");
    }

    #[test]
    fn converts_markdown_to_bbcode() {
        let md = "## 1.4.2\n- **Fixed** [a bug](https://x.y)\n- `code`\n\nText";
        assert_eq!(
            bbcode(md),
            "[size=5][b]1.4.2[/b][/size]\n[list]\n[*][b]Fixed[/b] [url=https://x.y]a bug[/url]\n[*][font=monospace]code[/font]\n[/list]\n\nText\n"
        );
    }
}
