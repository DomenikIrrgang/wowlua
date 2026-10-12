//! `.pkgmeta` (the BigWigs packager's project file): reading it, and
//! converting its externals into dependencies (spec CLI §12.6, step 6).
//! External URLs are interpreted like the packager does: old CurseForge
//! URLs, SVN `trunk` / `tags` paths, sub-paths of CurseForge and GitHub git
//! URLs, `.zip` archives, and git otherwise.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;

/// Where an external comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalSource {
    Git {
        url: String,
        tag: Option<String>,
        branch: Option<String>,
        commit: Option<String>,
        subdir: Option<String>,
    },
    Svn {
        url: String,
    },
    Zip {
        url: String,
        subdir: Option<String>,
    },
    Hg {
        url: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    /// The folder the packager puts it in (`libs/LibStub`).
    pub dir: String,
    pub source: ExternalSource,
    /// The CurseForge project slug (`curse-slug`, or from the URL).
    pub slug: Option<String>,
    /// `tag: latest`: the newest tag for git, trunk for SVN.
    pub latest: bool,
    /// The folder of the external a `move-folders` entry moves up.
    pub moved_up: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PkgMeta {
    /// The file name (`.pkgmeta`, `pkgmeta.yaml`, …).
    pub file: String,
    pub package_as: Option<String>,
    pub externals: Vec<External>,
    pub ignore: Vec<String>,
    pub plain_copy: Vec<String>,
    /// `manual-changelog`: file name and markup type.
    pub manual_changelog: Option<(String, Option<String>)>,
    pub move_folders: Vec<(String, String)>,
    pub enable_nolib: bool,
    /// `required-dependencies`, … as (slug, CurseForge relation type).
    pub relations: Vec<(String, String)>,
    /// `license-output`: the license file copied into `move-folders` targets.
    pub license_output: Option<String>,
    /// `changelog-title`: the title of the generated changelog.
    pub changelog_title: Option<String>,
    /// The `ignore` lists of the flavors whose packager runs use their own
    /// project file (`-g <game> -m <file>` in CI workflows), by flavor id;
    /// flavors packaged with this file are listed with its `ignore`.
    pub games: BTreeMap<String, Vec<String>>,
    /// The CI workflows package one game per packager run (`-g`), which
    /// applies the packager's flavor blocks.
    pub per_game: bool,
    /// The flavors those runs release.
    pub per_game_flavors: Vec<String>,
    /// The packager makes a TOC per game from `## Interface-<Type>:` lines
    /// (`enable-toc-creation: yes`, or `-S`).
    pub split: bool,
    /// The packager writes LF line endings (every run passes `-u`);
    /// otherwise its default, CR LF.
    pub unix: bool,
    /// What every packager run passes as `-n` (name template) and `-p` /
    /// `-w` / `-a` (project IDs).
    pub run_name: Option<String>,
    pub run_curseforge: Option<String>,
    pub run_wowinterface: Option<String>,
    pub run_wago: Option<String>,
}

/// The project file names the packager looks for.
pub const FILES: &[&str] = &[".pkgmeta", "pkgmeta.yaml", "pkgmeta.yml", ".pkgmeta.yaml"];

/// Reads the project's `.pkgmeta`, if it has one.
pub fn read(dir: &Path) -> Result<Option<PkgMeta>, Diagnostic> {
    for name in FILES {
        let path = dir.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut meta = parse(&text).map_err(|e| {
                Diagnostic::plain_error(format!("cannot read `{name}`: {e}")).with_path(&path)
            })?;
            meta.file = (*name).to_string();
            return Ok(Some(meta));
        }
    }
    Ok(None)
}

/// A `key: value` line split like the packager's `yaml_keyvalue`: the key
/// is the text before the first `:`; the value has its leading spaces
/// trimmed, booleans normalized to `yes` / `no`, and one leading and one
/// trailing quote removed.
/// Reads the project file of a packager project: `explicit` (`init
/// --from-pkgmeta <file>`), else one of [`FILES`], else the file its CI
/// workflows pass to the packager with `-m`. Workflow runs that package a
/// game (`-g`) with another file give that game's flavor its own `ignore`.
pub fn read_project(dir: &Path, explicit: Option<&Path>) -> Result<Option<PkgMeta>, Diagnostic> {
    let all_runs = workflow_runs(dir);
    let runs = single_game_runs(dir);
    let default = FILES
        .iter()
        .find(|f| dir.join(f).is_file())
        .map(|f| (*f).to_string());
    let main = match explicit {
        Some(path) => Some(
            path.strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/"),
        ),
        None => default.clone().or_else(|| {
            let retail = runs.iter().find(|r| {
                r.game.as_deref().and_then(crate::keywords::game_flavor) == Some("standard")
            });
            retail
                .or(all_runs.iter().find(|r| r.file.is_some()))
                .and_then(|r| r.file.clone())
        }),
    };
    let Some(main) = main else {
        return Ok(None);
    };
    let load = |name: &str| -> Result<PkgMeta, Diagnostic> {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            Diagnostic::plain_error(format!("cannot read `{name}`: {e}")).with_path(&path)
        })?;
        let mut meta = parse(&text).map_err(|e| {
            Diagnostic::plain_error(format!("cannot read `{name}`: {e}")).with_path(&path)
        })?;
        meta.file = name.to_string();
        Ok(meta)
    };
    let mut meta = load(&main)?;
    meta.per_game = !runs.is_empty();
    for run in &runs {
        if let Some(f) = run.game.as_deref().and_then(crate::keywords::game_flavor)
            && !meta.per_game_flavors.iter().any(|x| x == f)
        {
            meta.per_game_flavors.push(f.to_string());
        }
    }
    meta.split |= all_runs.iter().any(|r| r.split);
    meta.unix = !all_runs.is_empty() && all_runs.iter().all(|r| r.unix);
    // A value every run passes alike.
    let common = |get: fn(&WorkflowRun) -> &Option<String>| -> Option<String> {
        let first = get(all_runs.first()?).clone()?;
        all_runs
            .iter()
            .all(|r| get(r).as_deref() == Some(first.as_str()))
            .then_some(first)
    };
    meta.run_name = common(|r| &r.name);
    meta.run_curseforge = common(|r| &r.curseforge);
    meta.run_wowinterface = common(|r| &r.wowinterface);
    meta.run_wago = common(|r| &r.wago);
    for run in &runs {
        let Some(flavor) = run.game.as_deref().and_then(crate::keywords::game_flavor) else {
            continue;
        };
        let file = run.file.clone().or_else(|| default.clone());
        let ignore = match file {
            Some(f) if f != main => match load(&f) {
                Ok(other) => other.ignore,
                Err(_) => continue,
            },
            _ => meta.ignore.clone(),
        };
        meta.games.entry(flavor.to_string()).or_insert(ignore);
    }
    // Only worth keeping when the flavors differ.
    if meta.games.values().all(|i| *i == meta.ignore) {
        meta.games.clear();
    }
    Ok(Some(meta))
}

/// A packager run in a CI workflow: its game (`-g`), project file (`-m`)
/// and whether it makes a TOC per game (`-S`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowRun {
    pub game: Option<String>,
    pub file: Option<String>,
    pub split: bool,
    /// `-u`: Unix line endings.
    pub unix: bool,
    /// `-n`: the zip name and upload label template.
    pub name: Option<String>,
    /// `-p`, `-w`, `-a`: CurseForge, WoWInterface and Wago project IDs.
    pub curseforge: Option<String>,
    pub wowinterface: Option<String>,
    pub wago: Option<String>,
}

/// The packager's options that take a value, as its `getopts` reads them.
const VALUE_OPTIONS: &str = "pwartgmn";

/// A packager command line (`-zg retail -m .pkgmeta.yml`).
fn parse_run(args: &str) -> WorkflowRun {
    let tokens: Vec<String> = args
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '"' || c == '\'').to_string())
        .collect();
    let mut run = WorkflowRun::default();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        let Some(flags) = token.strip_prefix('-').filter(|f| !f.starts_with('-')) else {
            continue;
        };
        for (at, flag) in flags.char_indices() {
            if !VALUE_OPTIONS.contains(flag) {
                match flag {
                    'S' => run.split = true,
                    'u' => run.unix = true,
                    _ => {}
                }
                continue;
            }
            // The value is the rest of the token, else the next token.
            let rest = &flags[at + flag.len_utf8()..];
            let value = if rest.is_empty() {
                i += 1;
                tokens.get(i - 1).cloned()
            } else {
                Some(rest.to_string())
            };
            match flag {
                'g' => run.game = value,
                'm' => run.file = value.map(|v| v.trim_start_matches("./").to_string()),
                'n' => run.name = value,
                'p' => run.curseforge = value,
                'w' => run.wowinterface = value,
                'a' => run.wago = value,
                _ => {}
            }
            break;
        }
    }
    run
}

/// The packager runs of the project's GitHub workflows: each step using
/// the packager action (with its `args:`), and each `release.sh` command.
pub fn workflow_runs(dir: &Path) -> Vec<WorkflowRun> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(".github/workflows"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|e| {
                        e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml")
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut runs: Vec<WorkflowRun> = Vec::new();
    let add = |run: WorkflowRun, runs: &mut Vec<WorkflowRun>| {
        if !runs.contains(&run) {
            runs.push(run);
        }
    };
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        // The packager action step being read.
        let mut step: Option<WorkflowRun> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- ")
                && let Some(run) = step.take()
            {
                add(run, &mut runs);
            }
            if trimmed.contains("BigWigsMods/packager") {
                step = Some(WorkflowRun::default());
            } else if let Some(run) = &mut step
                && let Some(args) = trimmed.strip_prefix("args:")
            {
                *run = parse_run(args);
            } else if let Some(i) = trimmed.find("release.sh") {
                add(parse_run(&trimmed[i + "release.sh".len()..]), &mut runs);
            }
        }
        if let Some(run) = step.take() {
            add(run, &mut runs);
        }
    }
    runs
}

/// The workflow runs when every one packages a single game (`-g`): the
/// project is released game by game. Empty otherwise.
pub fn single_game_runs(dir: &Path) -> Vec<WorkflowRun> {
    let runs = workflow_runs(dir);
    if runs.iter().all(|r| {
        r.game
            .as_deref()
            .and_then(crate::keywords::game_flavor)
            .is_some()
    }) {
        runs
    } else {
        Vec::new()
    }
}

fn key_value(line: &str) -> (&str, String) {
    let (key, rest) = line.split_once(':').unwrap_or((line, ""));
    let mut value = rest.trim_start_matches(' ').to_string();
    match value.to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" => value = "yes".into(),
        "no" | "false" | "off" => value = "no".into(),
        _ => {}
    }
    (key, unquote(&value).to_string())
}

/// One leading and one trailing quote removed (`yaml_listitem`, the end of
/// `yaml_keyvalue`).
fn unquote(s: &str) -> &str {
    let s = s.strip_prefix(['\'', '"']).unwrap_or(s);
    s.strip_suffix(['\'', '"']).unwrap_or(s)
}

/// A list item like the packager's `yaml_listitem`: after the `-`, leading
/// spaces and one pair of quotes are trimmed; an inline comment stays part
/// of the item.
fn list_item(line: &str) -> String {
    let item = line
        .strip_prefix('-')
        .unwrap_or(line)
        .trim_start_matches(' ');
    unquote(item).to_string()
}

/// An external's fields as the packager collects them.
#[derive(Default)]
struct ExternalSpec {
    dir: String,
    url: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
    kind: Option<String>,
    slug: Option<String>,
    path: Option<String>,
}

/// Reads a `.pkgmeta` line by line, like the packager's own reader rather
/// than a YAML parser: comment lines are skipped; a line starting without a
/// space and holding a `:` starts a section; indented lines are list items
/// (`- item`) or `key: value` entries of that section. Unindented list
/// items are not part of any section, as for the packager.
pub fn parse(text: &str) -> Result<PkgMeta, String> {
    let mut meta = PkgMeta::default();
    let mut phase = String::new();
    let mut pending: Option<ExternalSpec> = None;
    let flush = |pending: &mut Option<ExternalSpec>, meta: &mut PkgMeta| {
        if let Some(spec) = pending.take()
            && let Some(e) = external(&spec)
        {
            meta.externals.push(e);
        }
    };
    for (n, raw) in text.split('\n').enumerate() {
        if raw.trim_start().starts_with('#') {
            continue;
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        if line[..indent].contains('\t') && !line.trim().is_empty() {
            return Err(format!(
                "line {} is indented with a tab; only spaces are allowed",
                n + 1
            ));
        }
        if !line.starts_with(' ') && !line.is_empty() {
            if !line.contains(':') {
                continue;
            }
            flush(&mut pending, &mut meta);
            let (key, value) = key_value(line);
            phase = key.to_string();
            let value = (!value.is_empty()).then_some(value);
            match key {
                "package-as" => meta.package_as = value,
                "enable-nolib-creation" => meta.enable_nolib = value.as_deref() == Some("yes"),
                "manual-changelog" => {
                    meta.manual_changelog = Some((value.unwrap_or_default(), None));
                }
                "license-output" => meta.license_output = value,
                "enable-toc-creation" => meta.split = value.as_deref() == Some("yes"),
                "changelog-title" => meta.changelog_title = value,
                _ => {}
            }
            continue;
        }
        if !line.starts_with(' ') {
            continue;
        }
        let item = line.trim_start_matches(' ');
        // `ignore` and `plain-copy` take any indented line holding `- `.
        if matches!(phase.as_str(), "ignore" | "plain-copy") {
            if line.contains("- ") {
                let target = if phase == "ignore" {
                    &mut meta.ignore
                } else {
                    &mut meta.plain_copy
                };
                target.push(list_item(item));
            }
            continue;
        }
        if item.starts_with("- ") {
            let kind = match phase.as_str() {
                "required-dependencies" => "requiredDependency",
                "optional-dependencies" => "optionalDependency",
                "embedded-libraries" => "embeddedLibrary",
                "tools-used" => "tool",
                _ => continue,
            };
            let slug = list_item(item).to_ascii_lowercase();
            meta.relations.retain(|(s, _)| *s != slug);
            meta.relations.push((slug, kind.to_string()));
            continue;
        }
        if !item.contains(':') {
            continue;
        }
        let (key, value) = key_value(item);
        match phase.as_str() {
            "manual-changelog" => {
                let entry = meta.manual_changelog.get_or_insert_with(Default::default);
                match key {
                    "filename" => entry.0 = value,
                    "markup-type" => entry.1 = Some(value),
                    _ => {}
                }
            }
            "move-folders" => meta.move_folders.push((key.to_string(), value)),
            "externals" => {
                let mut spare = ExternalSpec::default();
                let spec = pending.as_mut().unwrap_or(&mut spare);
                let slot = match key {
                    "url" => Some(&mut spec.url),
                    "tag" => Some(&mut spec.tag),
                    "branch" => Some(&mut spec.branch),
                    "commit" => Some(&mut spec.commit),
                    "type" => Some(&mut spec.kind),
                    "curse-slug" => Some(&mut spec.slug),
                    "path" => Some(&mut spec.path),
                    _ => None,
                };
                if let Some(slot) = slot {
                    *slot = Some(value);
                    continue;
                }
                // A new external; a value on the same line is its URL.
                flush(&mut pending, &mut meta);
                pending = Some(ExternalSpec {
                    dir: key.to_string(),
                    url: (!value.is_empty()).then_some(value),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    flush(&mut pending, &mut meta);
    // A `manual-changelog:` without a file name names no changelog.
    if meta
        .manual_changelog
        .as_ref()
        .is_some_and(|(f, _)| f.is_empty())
    {
        meta.manual_changelog = None;
    }
    Ok(meta)
}

/// One external, interpreted like the packager's `process_external`.
fn external(spec: &ExternalSpec) -> Option<External> {
    let dir = spec.dir.as_str();
    let mut uri = spec.url.clone()?;
    // Trailing comments, spaces and slashes.
    if let Some(i) = uri.find('#') {
        uri.truncate(i);
    }
    let mut uri = uri.trim().trim_end_matches('/').to_string();
    let mut tag = spec.tag.clone();
    let branch = spec.branch.clone();
    let commit = spec.commit.clone();
    let mut kind = spec.kind.clone();
    let mut slug = spec.slug.clone();
    let mut path = spec.path.clone();

    // Old CurseForge repository URLs.
    let lower = uri.to_ascii_lowercase();
    if lower.contains("git.curseforge.com") || lower.contains("git.wowace.com") {
        kind = Some("git".into());
        let rest = uri.trim_end_matches("/mainline.git");
        uri = format!(
            "https://repos{}",
            rest.split_once("://git").map_or(rest, |(_, r)| r)
        );
    } else if lower.contains("svn.curseforge.com") || lower.contains("svn.wowace.com") {
        kind = Some("svn".into());
        let rest = uri.replacen("/mainline", "", 1);
        uri = format!(
            "https://repos{}",
            rest.split_once("://svn").map_or(rest.as_str(), |(_, r)| r)
        );
    } else if lower.contains("hg.curseforge.com") || lower.contains("hg.wowace.com") {
        kind = Some("hg".into());
        let rest = uri.trim_end_matches("/mainline");
        uri = format!(
            "https://repos{}",
            rest.split_once("://hg").map_or(rest, |(_, r)| r)
        );
    } else if lower.starts_with("svn:") {
        kind = Some("svn".into());
    } else if (lower.ends_with(".zip") || lower.contains(".zip?")) && kind.is_none() {
        kind = Some("zip".into());
    }
    let mut kind = kind.unwrap_or_else(|| "git".into());

    for host in [
        "https://repos.curseforge.com/wow/",
        "https://repos.wowace.com/wow/",
    ] {
        if let Some(rest) = uri.strip_prefix(host) {
            let (project, sub) = rest.split_once('/').unwrap_or((rest, ""));
            if slug.is_none() {
                slug = Some(project.to_string());
            }
            if sub.starts_with("trunk") || sub.starts_with("tags/") {
                kind = "svn".into();
            }
        }
    }
    let latest = tag.as_deref() == Some("latest");
    let source = match kind.as_str() {
        "svn" => {
            // wowlua follows trunk for an SVN `tag: latest`.
            if latest {
                tag = None;
            }
            // `tag: X` on a trunk URL names `…/tags/X/<sub>`.
            if let Some(t) = &tag {
                let (trunk, sub) = match uri.split_once("/trunk/") {
                    Some((base, sub)) => (format!("{base}/trunk"), Some(sub.to_string())),
                    None => (uri.clone(), None),
                };
                if let Some(base) = trunk.strip_suffix("/trunk") {
                    uri = match sub {
                        Some(s) => format!("{base}/tags/{t}/{s}"),
                        None => format!("{base}/tags/{t}"),
                    };
                }
            }
            ExternalSource::Svn { url: uri }
        }
        "hg" => ExternalSource::Hg { url: uri },
        "zip" => ExternalSource::Zip {
            url: uri,
            subdir: path,
        },
        _ => {
            // Sub-paths of URLs whose layout is known.
            if let Some(s) = &slug
                && let Some(idx) = uri.find(&format!("/wow/{s}/"))
            {
                path = Some(uri[idx + format!("/wow/{s}/").len()..].to_string());
                uri.truncate(idx + format!("/wow/{s}").len());
            } else if let Some(rest) = uri.strip_prefix("https://github.com/") {
                let parts: Vec<&str> = rest.splitn(3, '/').collect();
                if parts.len() == 3 {
                    path = Some(parts[2].to_string());
                    uri = format!("https://github.com/{}/{}", parts[0], parts[1]);
                }
            }
            ExternalSource::Git {
                url: uri,
                tag,
                branch,
                commit,
                subdir: path,
            }
        }
    };
    Some(External {
        dir: dir.trim_matches('/').replace('\\', "/"),
        source,
        slug,
        latest,
        moved_up: None,
    })
}

/// A dependency name for an external folder: `libs/AceAddon-3.0` →
/// `aceaddon-3-0`.
pub fn dependency_name(dir: &str) -> String {
    let last = dir.rsplit('/').next().unwrap_or(dir);
    let mut out = String::new();
    for c in last.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.len() < 2 {
        format!("lib-{out}")
    } else {
        out.chars().take(64).collect()
    }
}

/// The folder that contains every external (`libs`), if they share one.
pub fn libs_dir(externals: &[External]) -> Option<String> {
    let parents: Vec<Vec<&str>> = externals
        .iter()
        .map(|e| {
            let parts: Vec<&str> = e.dir.split('/').collect();
            parts[..parts.len().saturating_sub(1)].to_vec()
        })
        .collect();
    let first = parents.first()?;
    let mut common = first.len();
    for p in &parents[1..] {
        common = common.min(p.iter().zip(first).take_while(|(a, b)| a == b).count());
    }
    (common > 0).then(|| first[..common].join("/"))
}

/// The dependency entry of an external (without `files`, `flavors` and
/// `install-path`, which the caller adds). `None` with a warning when it
/// cannot be converted.
pub fn dependency_entry(
    e: &External,
    sha256: impl Fn(&str) -> Result<String, String>,
) -> Result<Map<String, Value>, Diagnostic> {
    let mut m = Map::new();
    match &e.source {
        ExternalSource::Git {
            url,
            tag,
            branch,
            commit,
            subdir,
        } => {
            m.insert("git".into(), json!(url));
            if let Some(t) = tag {
                m.insert("tag".into(), json!(t));
            } else if let Some(b) = branch {
                m.insert("branch".into(), json!(b));
            } else if let Some(c) = commit {
                m.insert("rev".into(), json!(c));
            }
            if let Some(s) = subdir {
                m.insert("subdir".into(), json!(s));
            }
            // The packager fetches submodules of git externals.
            m.insert("submodules".into(), json!(true));
            if let Some(up) = &e.moved_up {
                m.insert("moved-up".into(), json!(up));
            }
        }
        ExternalSource::Svn { url } => {
            m.insert("svn".into(), json!(url));
        }
        ExternalSource::Zip { url, subdir } => {
            m.insert("archive".into(), json!(url));
            let sum = sha256(url).map_err(|err| {
                Diagnostic::plain_warning(format!(
                    "external `{}`: cannot download `{url}` to compute its checksum ({err}); not converted",
                    e.dir
                ))
            })?;
            m.insert("sha256".into(), json!(sum));
            if let Some(s) = subdir {
                m.insert("strip-prefix".into(), json!(s));
            }
            if let Some(up) = &e.moved_up {
                m.insert("moved-up".into(), json!(up));
            }
        }
        ExternalSource::Hg { url } => {
            return Err(Diagnostic::warning(
                "W4333",
                format!(
                    "external `{}` is a Mercurial repository (`{url}`), which wowlua cannot fetch; not converted",
                    e.dir
                ),
            )
            .with_help("replace it with a git mirror or a release archive"));
        }
    }
    m.insert("kind".into(), json!("lua-library"));
    if let Some(s) = &e.slug {
        m.insert("curseforge-slug".into(), json!(s));
    }
    Ok(m)
}

/// Keys of a dependency entry that only `.pkgmeta` may set in compatibility
/// mode (spec §12.6, E4005).
const SOURCE_KEYS: &[&str] = &[
    "version",
    "registry",
    "git",
    "tag",
    "branch",
    "rev",
    "subdir",
    "submodules",
    "svn",
    "path",
    "archive",
    "strip-prefix",
    "install-path",
];

/// `compat.pkgmeta` (spec CLI §12.6): dependencies and settings come from
/// `.pkgmeta`; the manifest only adds wowlua-specific settings (`files`,
/// `flavors`, `sha256` of zip externals, …). Keys both define are E4005.
pub fn apply_compat(
    manifest: &mut wowlua_manifest::LoadedManifest,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let meta = match read(&manifest.root) {
        Ok(Some(m)) => m,
        Ok(None) => {
            diagnostics.push(
                Diagnostic::plain_error("`compat.pkgmeta` is set, but there is no `.pkgmeta`")
                    .with_path(&manifest.path),
            );
            return;
        }
        Err(d) => {
            diagnostics.push(d);
            return;
        }
    };
    let conflict = |what: String| {
        Diagnostic::error(
            "E4005",
            format!("{what} is set by both `.pkgmeta` and wowlua.json"),
        )
        .with_path(&manifest.path)
        .with_help("with `compat.pkgmeta`, keep it in `.pkgmeta` and remove it from wowlua.json")
    };
    let m = &mut manifest.manifest;
    let libs_dir = libs_dir(&meta.externals);
    let mut deps = indexmap::IndexMap::new();
    for e in &meta.externals {
        let name = dependency_name(&e.dir);
        let own = m.dependencies.shift_remove(&name);
        let mut entry = match dependency_entry(e, |_| Err("set `sha256` in wowlua.json".into())) {
            Ok(entry) => entry,
            Err(d) if matches!(e.source, ExternalSource::Zip { .. }) => {
                let _ = d;
                let mut m = Map::new();
                if let ExternalSource::Zip { url, subdir } = &e.source {
                    m.insert("archive".into(), json!(url));
                    if let Some(s) = subdir {
                        m.insert("strip-prefix".into(), json!(s));
                    }
                }
                m.insert("kind".into(), json!("lua-library"));
                m
            }
            Err(d) => {
                diagnostics.push(d);
                continue;
            }
        };
        let install = match &libs_dir {
            Some(d) => e.dir[d.len() + 1..].to_string(),
            None => e.dir.clone(),
        };
        entry.insert("install-path".into(), json!(install));
        if let Some(own) = own {
            let own_value = serde_json::to_value(&own).unwrap_or_default();
            match own_value {
                Value::Object(map) => {
                    for (k, v) in map {
                        if SOURCE_KEYS.contains(&k.as_str()) {
                            diagnostics.push(conflict(format!("`dependencies.{name}.{k}`")));
                        } else {
                            entry.insert(k, v);
                        }
                    }
                }
                _ => diagnostics.push(conflict(format!("`dependencies.{name}`"))),
            }
        }
        match serde_json::from_value::<wowlua_manifest::model::DependencyDetail>(Value::Object(
            entry,
        )) {
            Ok(d) => {
                deps.insert(
                    name,
                    wowlua_manifest::model::Dependency::Detailed(Box::new(d)),
                );
            }
            Err(err) => diagnostics.push(
                Diagnostic::plain_error(format!("external `{}`: {err}", e.dir))
                    .with_path(&manifest.path),
            ),
        }
    }
    for (k, v) in std::mem::take(&mut m.dependencies) {
        let sourceless = matches!(&v, wowlua_manifest::model::Dependency::Detailed(d)
            if d.git.is_none() && d.svn.is_none() && d.path.is_none() && d.archive.is_none() && d.version.is_none());
        if sourceless {
            diagnostics.push(
                Diagnostic::error(
                    "E4002",
                    format!("dependency `{k}` is not a `.pkgmeta` external and names no source"),
                )
                .with_path(&manifest.path),
            );
        }
        deps.insert(k, v);
    }
    m.dependencies = deps;
    if !meta.externals.is_empty() {
        let wanted = libs_dir.clone().unwrap_or_else(|| ".".into());
        if m.build.libs_dir != "libs" && m.build.libs_dir != wanted {
            diagnostics.push(conflict("`build.libs-dir`".into()));
        }
        m.build.libs_dir = wanted;
    }
    if !meta.ignore.is_empty() {
        if !m.build.exclude.is_empty() {
            diagnostics.push(conflict("`build.exclude` (`ignore`)".into()));
        }
        m.build.exclude = meta
            .ignore
            .iter()
            .flat_map(|p| {
                let p = p.trim_end_matches("/*").trim_end_matches('/').to_string();
                [p.clone(), format!("{p}/**")]
            })
            .collect();
    }
    if let Some(name) = &meta.package_as {
        let addon = m.addon.get_or_insert_with(Default::default);
        if addon.name.as_ref().is_some_and(|n| n != name) {
            diagnostics.push(conflict("`addon.name` (`package-as`)".into()));
        }
        addon.name = Some(name.clone());
    }
    if let Some((file, _)) = &meta.manual_changelog {
        let publish = m.publish.get_or_insert_with(Default::default);
        if publish.changelog.is_some() {
            diagnostics.push(conflict("`publish.changelog` (`manual-changelog`)".into()));
        }
        publish.changelog = Some(Some(file.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BARTENDER: &str = "package-as: Bartender4\nexternals:\n  libs/LibStub: https://repos.wowace.com/wow/libstub/tags/1.0\n  libs/CallbackHandler-1.0: https://repos.wowace.com/wow/callbackhandler/trunk/CallbackHandler-1.0\n  libs/LibActionButton-1.0: https://repos.wowace.com/wow/libactionbutton-1-0\n  libs/LibKeyBound-1.0:\n    url: https://repos.curseforge.com/wow/libkeybound-1-0/LibKeyBound-1.0\n    tag: v1.2\n  libs/LibClassicSpellActionCount-1.0: https://github.com/Ennea/LibClassicSpellActionCount-1.0.git # comment\n  libs/Old: svn://svn.wowace.com/wow/old/mainline/trunk\n  libs/Tagged:\n    url: https://repos.wowace.com/wow/ace3/trunk/AceAddon-3.0\n    tag: Release-r1300\n  libs/Zip: https://example.com/Lib.zip\n  libs/Hg: https://hg.wowace.com/wow/x/mainline\n\nignore:\n  - .luacheckrc\n  - locale/find-locale-strings.lua\nenable-nolib-creation: yes\nmanual-changelog:\n  filename: CHANGES.md\n  markup-type: markdown\nrequired-dependencies:\n  - masque\nmove-folders:\n  Bartender4/Options: Bartender4_Options\n";

    #[test]
    fn interprets_externals_like_the_packager() {
        let m = parse(BARTENDER).unwrap();
        assert_eq!(m.package_as.as_deref(), Some("Bartender4"));
        assert!(m.enable_nolib);
        assert_eq!(
            m.ignore,
            vec![".luacheckrc", "locale/find-locale-strings.lua"]
        );
        assert_eq!(
            m.manual_changelog,
            Some(("CHANGES.md".into(), Some("markdown".into())))
        );
        assert_eq!(
            m.relations,
            vec![("masque".into(), "requiredDependency".into())]
        );
        assert_eq!(
            m.move_folders,
            vec![("Bartender4/Options".into(), "Bartender4_Options".into())]
        );
        let src = |i: usize| m.externals[i].source.clone();
        assert_eq!(
            src(0),
            ExternalSource::Svn {
                url: "https://repos.wowace.com/wow/libstub/tags/1.0".into()
            }
        );
        assert_eq!(m.externals[0].slug.as_deref(), Some("libstub"));
        assert_eq!(
            src(1),
            ExternalSource::Svn {
                url: "https://repos.wowace.com/wow/callbackhandler/trunk/CallbackHandler-1.0"
                    .into()
            }
        );
        assert_eq!(
            src(2),
            ExternalSource::Git {
                url: "https://repos.wowace.com/wow/libactionbutton-1-0".into(),
                tag: None,
                branch: None,
                commit: None,
                subdir: None
            }
        );
        assert_eq!(
            src(3),
            ExternalSource::Git {
                url: "https://repos.curseforge.com/wow/libkeybound-1-0".into(),
                tag: Some("v1.2".into()),
                branch: None,
                commit: None,
                subdir: Some("LibKeyBound-1.0".into())
            }
        );
        assert_eq!(
            src(4),
            ExternalSource::Git {
                url: "https://github.com/Ennea/LibClassicSpellActionCount-1.0.git".into(),
                tag: None,
                branch: None,
                commit: None,
                subdir: None
            }
        );
        assert_eq!(
            src(5),
            ExternalSource::Svn {
                url: "https://repos.wowace.com/wow/old/trunk".into()
            }
        );
        assert_eq!(
            src(6),
            ExternalSource::Svn {
                url: "https://repos.wowace.com/wow/ace3/tags/Release-r1300/AceAddon-3.0".into()
            }
        );
        assert!(matches!(src(7), ExternalSource::Zip { .. }));
        assert!(matches!(src(8), ExternalSource::Hg { .. }));
        assert_eq!(libs_dir(&m.externals).as_deref(), Some("libs"));
        assert_eq!(dependency_name("libs/AceAddon-3.0"), "aceaddon-3-0");
        assert_eq!(dependency_name("libs/LibStub"), "libstub");
    }

    #[test]
    fn reads_lines_like_the_packager() {
        let text = "package-as: \"ATT\" \r\nignore: # comment\r\n  # - skipped\r\n  - *.md\r\n  - '*.meta.lua' # kept\r\n- unindented\r\nexternals:\n  libs/A: https://github.com/a/a\n  url: https://ignored\n  libs/B:\n    url: \"https://github.com/b/b\"\n    branch: main\nlicense-output: LICENSE.txt\nenable-nolib-creation: On\nmanual-changelog: CHANGES.txt\nrequired-dependencies:\n  - Masque\n";
        let m = parse(text).unwrap();
        assert_eq!(m.package_as.as_deref(), Some("ATT\" "));
        assert_eq!(m.ignore, vec!["*.md", "*.meta.lua' # kept"]);
        assert_eq!(m.license_output.as_deref(), Some("LICENSE.txt"));
        assert!(m.enable_nolib);
        assert_eq!(m.manual_changelog, Some(("CHANGES.txt".into(), None)));
        assert_eq!(
            m.relations,
            vec![("masque".into(), "requiredDependency".into())]
        );
        let dirs: Vec<_> = m.externals.iter().map(|e| e.dir.as_str()).collect();
        assert_eq!(dirs, vec!["libs/A", "libs/B"]);
        assert!(matches!(
            &m.externals[1].source,
            ExternalSource::Git { url, branch: Some(b), .. }
                if url == "https://github.com/b/b" && b == "main"
        ));
        assert!(parse("ignore:\n\t- x\n").is_err());
    }

    #[test]
    fn reads_packager_runs_of_workflows() {
        assert_eq!(
            parse_run(" -zug retail -m './.pkgmeta-retail' -S"),
            WorkflowRun {
                game: Some("retail".into()),
                file: Some(".pkgmeta-retail".into()),
                split: true,
                unix: true,
                ..Default::default()
            }
        );
        let dir = tempfile::tempdir().unwrap();
        let workflows = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        let step = |args: &str| {
            format!(
                "      - name: Package\n        uses: BigWigsMods/packager@v2\n{args}      - name: Next\n        run: true\n"
            )
        };
        std::fs::write(
            workflows.join("beta.yml"),
            step("        with:\n          args: -zg retail\n"),
        )
        .unwrap();
        // A release without `-g` packages every game in one run.
        std::fs::write(workflows.join("release.yml"), step("")).unwrap();
        assert_eq!(workflow_runs(dir.path()).len(), 2);
        assert!(single_game_runs(dir.path()).is_empty());
        std::fs::remove_file(workflows.join("release.yml")).unwrap();
        std::fs::write(
            workflows.join("classic.yml"),
            "    steps:\n      - run: curl -s https://x/release.sh | bash -s -- -g classic -m .pkgmeta-classic.yml\n",
        )
        .unwrap();
        let runs = single_game_runs(dir.path());
        let games: Vec<_> = runs.iter().filter_map(|r| r.game.as_deref()).collect();
        assert_eq!(games, ["retail", "classic"]);
    }
}
