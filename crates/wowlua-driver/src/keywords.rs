//! BigWigs packager keyword substitution (spec CLI §12.6, `compat.packager-
//! keywords`): a text preprocessing step for passthrough `.lua` and `.xml`
//! files. Repository keywords are replaced; inactive blocks are commented
//! out in the packager's style, so line numbers stay the same.

use std::path::Path;
use std::process::{Command, Stdio};

/// LF line endings, like `packager -u`: a CR before each LF and at the
/// end of the file is removed.
pub fn lf(bytes: &[u8]) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut lines = bytes.split(|b| *b == b'\n').peekable();
    while let Some(line) = lines.next() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        out.extend_from_slice(line);
        if lines.peek().is_some() {
            out.push(b'\n');
        }
    }
    out
}

/// Line endings as the packager's default `line_ending_filter` writes
/// them: CR LF after every line, a last line without newline kept so.
pub fn crlf(bytes: &[u8]) -> Vec<u8> {
    let normalized = lf(bytes);
    let mut out = Vec::with_capacity(normalized.len() + normalized.len() / 32);
    for b in normalized {
        if b == b'\n' {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

/// Text files the packager processes (keywords, line endings).
pub fn processed_extension(path: &str) -> bool {
    // The packager's `match_pattern`, case-sensitive like its shell globs.
    [".lua", ".xml", ".toc", ".md", ".txt"]
        .iter()
        .any(|e| path.ends_with(e))
}

/// Whether `path` matches a packager `plain-copy` pattern (shell
/// wildcards, `*` matching `/` too, case-insensitive).
pub fn plain_copy(patterns: &[String], paths: &[&str]) -> bool {
    patterns.iter().any(|p| {
        let p = p.trim_end_matches('/');
        [p.to_string(), format!("{p}/*")].iter().any(|pattern| {
            globset::GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map(|g| {
                    let m = g.compile_matcher();
                    paths.iter().any(|path| m.is_match(path))
                })
                .unwrap_or(false)
        })
    })
}

/// Values for repository and build keywords.
#[derive(Clone, Debug, Default)]
pub struct KeywordInfo {
    pub version: String,
    pub hash: String,
    pub short_hash: String,
    /// Commit count of `HEAD`.
    pub revision: String,
    pub author: String,
    /// `HEAD`'s commit time, Unix seconds.
    pub timestamp: i64,
    /// Build time, Unix seconds.
    pub build_timestamp: i64,
    /// The profile's `debug`.
    pub debug: bool,
    /// An untagged (alpha or dev) build.
    pub alpha: bool,
    /// The repository root, for `@file-*@` keywords.
    pub repo: Option<std::path::PathBuf>,
}

/// The build time: `SOURCE_DATE_EPOCH` when set (reproducible builds).
pub fn build_time() -> i64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(crate::host::unix_now)
}

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
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl KeywordInfo {
    /// Repository values from `git` (empty outside a repository).
    pub fn from_git(dir: &Path, version: &str, debug: bool, alpha: bool, now: i64) -> Self {
        // The packager's file type: untagged builds and tags naming
        // "alpha" are alpha builds (`-beta` tags are not).
        let tag = head_tag(dir);
        let alpha = match &tag {
            Some(t) => t.to_ascii_lowercase().contains("alpha"),
            None if git(dir, &["rev-parse", "HEAD"]).is_some() => true,
            None => alpha,
        };
        let mut info = KeywordInfo {
            version: tag
                .or_else(|| packager_version(dir))
                .unwrap_or_else(|| version.to_string()),
            debug,
            alpha,
            build_timestamp: now,
            ..Default::default()
        };
        if let Some(line) = git(
            dir,
            &["log", "-1", "--abbrev=7", "--format=%H%x09%h%x09%an%x09%at"],
        ) {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() == 4 {
                info.hash = parts[0].into();
                info.short_hash = parts[1].into();
                info.author = parts[2].into();
                info.timestamp = parts[3].parse().unwrap_or(0);
            }
            info.revision = git(dir, &["rev-list", "--count", "HEAD"]).unwrap_or_default();
            info.repo = Some(dir.to_path_buf());
        }
        info
    }
}

/// `@project-version@` as the packager computes it (decision T123): the
/// tag on `HEAD`, otherwise `git describe` with 7-digit hashes, ignoring
/// alpha tags, otherwise the short hash.
fn packager_version(dir: &Path) -> Option<String> {
    if let Some(tag) = head_tag(dir) {
        return Some(tag);
    }
    git(
        dir,
        &[
            "describe",
            "--tags",
            "--abbrev=7",
            "--exclude=*[Aa][Ll][Pp][Hh][Aa]*",
        ],
    )
    .filter(|d| !d.is_empty())
    .or_else(|| git(dir, &["describe", "--tags", "--always", "--abbrev=7"]))
}

/// The tag on `HEAD` the packager picks: the most recent one, or the one a
/// GitHub Actions tag build names (`GITHUB_REF_NAME`).
pub fn head_tag(dir: &Path) -> Option<String> {
    let tags = git(
        dir,
        &[
            "tag",
            "--points-at",
            "HEAD",
            "--sort=-committerdate",
            "--sort=-v:refname",
            "--sort=-taggerdate",
        ],
    )?;
    let tags: Vec<&str> = tags.lines().filter(|t| !t.is_empty()).collect();
    let named = std::env::var("GITHUB_REF")
        .ok()
        .filter(|r| r.starts_with("refs/tags/"))
        .and_then(|_| std::env::var("GITHUB_REF_NAME").ok())
        .filter(|n| tags.contains(&n.as_str()));
    named.or_else(|| tags.first().map(|t| t.to_string()))
}

/// `2026-10-12T04:45:27Z` and `20261012044527` of Unix seconds.
pub fn date_pair(secs: i64) -> (String, String) {
    dates(secs)
}

/// `2026-10-12T04:45:27Z` of Unix seconds.
pub(crate) fn iso_date(secs: i64) -> String {
    dates(secs).0
}

/// `2026-10-12T04:45:27Z` and `20261012044527` of Unix seconds.
fn dates(secs: i64) -> (String, String) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    (
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
        format!("{y:04}{m:02}{d:02}{h:02}{mi:02}{s:02}"),
    )
}

/// Comments out (or uncomments) a block in a Lua file like the packager's
/// `lua_filter`.
fn lua_filter(text: &str, keyword: &str, active: bool) -> String {
    let width = match keyword {
        "alpha" => "=",
        "debug" => "==",
        k if k == "retail" || k.starts_with("version-") => "====",
        _ => "===",
    };
    if active {
        return text.to_string();
    }
    text.replace(
        &format!("--@{keyword}@"),
        &format!("--[{width}[@{keyword}@"),
    )
    .replace(
        &format!("--@end-{keyword}@"),
        &format!("--@end-{keyword}@]{width}]"),
    )
    .replace(
        &format!("--[===[@non-{keyword}@"),
        &format!("--@non-{keyword}@"),
    )
    .replace(
        &format!("--@end-non-{keyword}@]===]"),
        &format!("--@end-non-{keyword}@"),
    )
}

/// The packager's `xml_filter`.
fn xml_filter(text: &str, keyword: &str, active: bool) -> String {
    if active {
        return text.to_string();
    }
    text.replace(&format!("<!--@{keyword}@-->"), &format!("<!--@{keyword}@"))
        .replace(
            &format!("<!--@end-{keyword}@-->"),
            &format!("@end-{keyword}@-->"),
        )
        .replace(
            &format!("<!--@non-{keyword}@"),
            &format!("<!--@non-{keyword}@-->"),
        )
        .replace(
            &format!("@end-non-{keyword}@-->"),
            &format!("<!--@end-non-{keyword}@-->"),
        )
}

/// The flavor keywords and the flavor each names.
pub const FLAVOR_KEYWORDS: &[(&str, &str)] = &[
    ("retail", "standard"),
    ("version-retail", "standard"),
    ("version-classic", "vanilla"),
    ("version-vanilla", "vanilla"),
    ("version-bcc", "tbc"),
    ("version-wrath", "wrath"),
    // Titan Reforged is not a flavor of its own: wrath builds are
    // `-g wrath` builds, where its blocks are inactive.
    ("version-titan", "titan"),
    ("version-cata", "cata"),
    ("version-mists", "mists"),
    ("version-forever", "camelot"),
];

/// An XML file of a nolib package: its `no-lib-strip` blocks commented out,
/// like the packager's nolib pass.
pub fn nolib_xml(text: &str) -> String {
    xml_filter(text, "no-lib-strip", false)
}

/// Whether `text` uses `@version-…@` blocks, which the packager rejects in
/// a multi-version build.
pub fn has_version_blocks(text: &str) -> bool {
    FLAVOR_KEYWORDS
        .iter()
        .filter(|(k, _)| k.starts_with("version-"))
        .any(|(k, _)| text.contains(&format!("@{k}@")) || text.contains(&format!("@non-{k}@")))
}

/// Whether `text` uses flavor blocks.
pub fn has_flavor_blocks(text: &str) -> bool {
    FLAVOR_KEYWORDS
        .iter()
        .any(|(k, _)| text.contains(&format!("@{k}@")) || text.contains(&format!("@non-{k}@")))
}

/// Deletes `@do-not-package@` blocks, lines included, like the packager.
fn remove_do_not_package(text: &str, xml: bool) -> String {
    let (start, end) = if xml {
        ("<!--@do-not-package@-->", "<!--@end-do-not-package@-->")
    } else {
        ("--@do-not-package@", "--@end-do-not-package@")
    };
    if !text.contains(start) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut removing = false;
    for line in text.split_inclusive('\n') {
        if removing {
            if line.contains(end) {
                removing = false;
            }
            continue;
        }
        if line.contains(start) {
            removing = true;
            continue;
        }
        out.push_str(line);
    }
    out
}

/// Repository keywords of the package and its blocks, the way the packager
/// processes the files of an external: debug blocks follow the profile,
/// `@do-not-package@` blocks go in release builds, and XML `no-lib-strip`
/// blocks are always commented out (the addon embeds its libraries itself).
pub fn apply_library(text: &str, info: &KeywordInfo, xml: bool) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let mut out = substitute(text, info);
    if !info.debug {
        out = remove_do_not_package(&out, xml);
    }
    let filter = if xml { xml_filter } else { lua_filter };
    out = filter(&out, "debug", info.debug);
    if xml {
        out = xml_filter(&out, "no-lib-strip", false);
    }
    out
}

/// Keywords whose values of an SVN library need `svn info` / `svn log`.
pub const SVN_DETAIL_KEYWORDS: &[&str] = &[
    "@project-author@",
    "@project-version@",
    "@project-revision@",
    "@project-date-iso@",
    "@project-date-integer@",
    "@project-timestamp@",
];

/// The `@file-*@` keywords of one file.
pub fn substitute_file(text: &str, f: &wowlua_pkg::fetch::FileVcs) -> String {
    let (iso, int) = dates(f.timestamp);
    text.replace("@file-revision@", &f.revision)
        .replace("@file-hash@", &f.hash)
        .replace("@file-abbreviated-hash@", &f.short_hash)
        .replace("@file-author@", &f.author)
        .replace("@file-date-iso@", &iso)
        .replace("@file-date-integer@", &int)
        .replace("@file-timestamp@", &f.timestamp.to_string())
}

pub fn substitute(text: &str, info: &KeywordInfo) -> String {
    let (date_iso, date_int) = dates(info.timestamp);
    let (build_iso, build_int) = dates(info.build_timestamp);
    text.replace("@project-revision@", &info.revision)
        .replace("@project-hash@", &info.hash)
        .replace("@project-abbreviated-hash@", &info.short_hash)
        .replace("@project-author@", &info.author)
        .replace("@project-date-iso@", &date_iso)
        .replace("@project-date-integer@", &date_int)
        .replace("@project-timestamp@", &info.timestamp.to_string())
        .replace("@project-version@", &info.version)
        .replace("@build-date@", &build_iso[..10])
        .replace("@build-date-iso@", &build_iso)
        .replace("@build-date-integer@", &build_int)
        .replace("@build-timestamp@", &info.build_timestamp.to_string())
}

/// Applies repository keywords and the debug/alpha blocks; release builds
/// drop `@do-not-package@` blocks. `rel` is the file's path in the
/// repository, for `@file-*@`.
/// Repository keywords of a text file of the project (`.md`, `.txt` and
/// the like get nothing else from the packager).
pub fn apply_text(text: &str, info: &KeywordInfo, rel: &Path, original: &Path) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let mut out = substitute(text, info);
    if out.contains("@file-")
        && let Some(repo) = &info.repo
    {
        // History follows renames; files `init` moved but nobody committed
        // yet have it under their original path.
        let log = |path: &str| {
            git(
                repo,
                &[
                    "log",
                    "-1",
                    "--follow",
                    "--abbrev=7",
                    "--format=%H%x09%h%x09%an%x09%at",
                    "--",
                    path,
                ],
            )
            .filter(|l| !l.is_empty())
        };
        let mut path = rel.to_string_lossy().into_owned();
        if log(&path).is_none() {
            path = original.to_string_lossy().into_owned();
        }
        let line = git(
            repo,
            &[
                "log",
                "-1",
                "--abbrev=7",
                "--format=%H%x09%h%x09%an%x09%at",
                "--",
                &path,
            ],
        )
        .unwrap_or_default();
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 4 {
            let ts: i64 = parts[3].parse().unwrap_or(0);
            // The commit count of the file's last commit, like the
            // packager's `rev-list --count <file hash>`.
            let count = git(repo, &["rev-list", "--count", parts[0]]).unwrap_or_default();
            out = substitute_file(
                &out,
                &wowlua_pkg::fetch::FileVcs {
                    revision: count,
                    hash: parts[0].into(),
                    short_hash: parts[1].into(),
                    author: parts[2].into(),
                    timestamp: ts,
                },
            );
        }
    }
    out
}

pub fn apply(text: &str, info: &KeywordInfo, rel: &Path, original: &Path, xml: bool) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let mut out = apply_text(text, info, rel, original);
    if !info.debug {
        out = remove_do_not_package(&out, xml);
    }
    let filter = if xml { xml_filter } else { lua_filter };
    out = filter(&out, "debug", info.debug);
    out = filter(&out, "alpha", info.alpha);
    out
}

/// Applies the flavor blocks for `flavor` (packager game types).
pub fn apply_flavor(text: &str, flavor: &str, xml: bool) -> String {
    let filter = if xml { xml_filter } else { lua_filter };
    let mut out = text.to_string();
    for (keyword, f) in FLAVOR_KEYWORDS {
        out = filter(&out, keyword, *f == flavor);
    }
    out
}

/// The TOC a packaged (release) build of the packager uses: `#@do-not-package@`
/// blocks removed, `#@debug@` and `#@alpha@` blocks removed and their
/// `#@non-…@` blocks uncommented (the packager's `toc_filter`).
pub fn packaged_toc(text: &str) -> String {
    packaged_toc_for(text, None)
}

/// [`packaged_toc`] for one game (a flavor id): its flavor blocks
/// (`#@retail@`, `#@version-…@`) are kept, the other flavors' blocks
/// removed, and `#@non-…@` blocks of the others uncommented, like the
/// packager's single-game and split builds.
pub fn packaged_toc_for(text: &str, game: Option<&str>) -> String {
    let mut keywords: Vec<(&str, bool)> = vec![
        ("do-not-package", false),
        ("debug", false),
        ("alpha", false),
    ];
    if let Some(game) = game {
        keywords.extend(FLAVOR_KEYWORDS.iter().map(|(k, f)| (*k, *f == game)));
    }
    let mut out = String::with_capacity(text.len());
    // Which block a line is in: (negated, active).
    let mut block: Option<(bool, bool)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let marker = |kw: &str| -> Option<(bool, bool)> {
            // (start, negated)
            for (tag, start, neg) in [
                (format!("#@{kw}@"), true, false),
                (format!("#@end-{kw}@"), false, false),
                (format!("#@non-{kw}@"), true, true),
                (format!("#@end-non-{kw}@"), false, true),
            ] {
                if trimmed.starts_with(&tag) {
                    return Some((start, neg));
                }
            }
            None
        };
        let mut handled = false;
        for (kw, active) in &keywords {
            if let Some((start, neg)) = marker(kw) {
                block = if start { Some((neg, *active)) } else { None };
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        match block {
            // Active blocks stay; inactive ones are dropped.
            Some((false, true)) => out.push_str(line),
            Some((false, false)) | Some((true, true)) => {}
            // The `non-` blocks of inactive keywords are uncommented.
            Some((true, false)) => {
                let rest = line.trim_start();
                match rest.strip_prefix('#') {
                    Some(r) if r.starts_with([' ', '\t']) => {
                        out.push_str(r.trim_start_matches([' ', '\t']))
                    }
                    _ => out.push_str(line),
                }
            }
            None => out.push_str(line),
        }
    }
    out
}

/// `@toc-version-<game>@` placeholders (the `NumyAddon/ToCVersions` action
/// many packager projects run first) replaced by the current interface
/// numbers of the flavor table.
pub fn toc_version_placeholders(text: &str) -> String {
    if !text.contains("@toc-version-") {
        return text.to_string();
    }
    let table = wowlua_flavors::table();
    let mut out = text.to_string();
    for (game, flavor) in [
        ("midnight", "standard"),
        ("retail", "standard"),
        ("camelot", "camelot"),
        ("forever", "camelot"),
        ("mists", "mists"),
        ("cata", "cata"),
        ("wrath", "wrath"),
        ("bcc", "tbc"),
        ("classic", "vanilla"),
    ] {
        if let Some(n) = table.flavor(flavor).and_then(|f| f.interface.first()) {
            out = out.replace(&format!("@toc-version-{game}@"), &n.to_string());
        }
    }
    out
}

/// The flavor of a packager `-g` value: a game type, or game versions
/// (`1.15.9`, `5.5.4,1.15.9`) of one game type; `None` for several types,
/// which the packager builds as a multi-version package.
pub fn game_flavor(game: &str) -> Option<&'static str> {
    if let Some(f) = interface_type_flavor(game) {
        return Some(f);
    }
    let mut flavor = None;
    for version in game.split(',') {
        let parts: Vec<&str> = version.trim().split('.').collect();
        let [major, minor, _] = parts.as_slice() else {
            return None;
        };
        let f = match (*major, *minor) {
            ("1", m) if m.len() == 2 && m.starts_with('6') => "camelot",
            ("1", _) => "vanilla",
            ("2", _) => "tbc",
            ("3", "80") => "titan",
            ("3", _) => "wrath",
            ("4", _) => "cata",
            ("5", _) => "mists",
            _ => "standard",
        };
        if flavor.is_some_and(|x| x != f) {
            return None;
        }
        flavor = Some(f);
    }
    // Titan Reforged builds have no flavor of their own.
    flavor.filter(|f| *f != "titan")
}

/// The game types of the packager's `## Interface-<Type>:` lines and the
/// flavors they name.
pub fn interface_type_flavor(kind: &str) -> Option<&'static str> {
    Some(match kind.to_ascii_lowercase().as_str() {
        "retail" | "mainline" => "standard",
        "classic" | "vanilla" => "vanilla",
        "bcc" | "tbc" => "tbc",
        "wrath" | "wotlkc" | "titan" => "wrath",
        "cata" => "cata",
        "mists" => "mists",
        "forever" | "camelot" => "camelot",
        _ => return None,
    })
}

/// An XML file with every packager block's negated side active: `@debug@`,
/// `@alpha@`, flavor and `no-lib-strip` blocks commented out, their
/// `@non-…@` blocks uncommented (what some packaged build loads).
pub fn negated_xml(text: &str) -> String {
    let mut out = text.to_string();
    for keyword in ["debug", "alpha", "no-lib-strip"]
        .into_iter()
        .chain(FLAVOR_KEYWORDS.iter().map(|(k, _)| *k))
    {
        out = xml_filter(&out, keyword, false);
    }
    out
}

/// An XML file as a packaged (release) build of the packager loads it:
/// `@do-not-package@` blocks removed, `@debug@` and `@alpha@` blocks
/// commented out and their `@non-…@` blocks uncommented.
pub fn packaged_xml(text: &str) -> String {
    let out = remove_do_not_package(text, true);
    let out = xml_filter(&out, "debug", false);
    xml_filter(&out, "alpha", false)
}

/// Keyword values of a fetched package (library or external addon), as
/// the packager computes them for an external. `detail` asks for values
/// that need the network (SVN author).
pub fn package_info(
    source: &wowlua_pkg::Source,
    pin: Option<&str>,
    debug: bool,
    detail: bool,
) -> KeywordInfo {
    let vcs = match (wowlua_pkg::Cache::new(), pin) {
        (Some(cache), Some(pin)) => wowlua_pkg::fetch::package_vcs(&cache, source, pin, detail),
        _ => Default::default(),
    };
    KeywordInfo {
        version: vcs.version,
        hash: vcs.hash,
        short_hash: vcs.short_hash,
        revision: vcs.revision,
        author: vcs.author,
        timestamp: vcs.timestamp,
        build_timestamp: build_time(),
        debug,
        alpha: false,
        repo: None,
    }
}

/// The `@file-*@` keywords of a file of an embedded library.
pub fn library_file_keywords(
    text: &str,
    lib: &crate::embed::LuaLib,
    info: &KeywordInfo,
    rel: &str,
    bytes: &[u8],
) -> String {
    let (Some(cache), Some(pin)) = (wowlua_pkg::Cache::new(), lib.pin.as_deref()) else {
        return text.to_string();
    };
    let project = wowlua_pkg::fetch::PackageVcs {
        version: info.version.clone(),
        hash: info.hash.clone(),
        short_hash: info.short_hash.clone(),
        revision: info.revision.clone(),
        author: info.author.clone(),
        timestamp: info.timestamp,
    };
    let file = wowlua_pkg::fetch::package_file_vcs(&cache, &lib.source, pin, rel, bytes, &project);
    substitute_file(text, &file)
}

/// `@localization(…)@`: without local files (`compat.localization`), W4030.
pub fn uses_localization(text: &str) -> bool {
    text.contains("@localization(")
}

/// Fills `--@localization(…)@` lines from local files (decision T491): the
/// keyword is replaced by `file_of(locale)`, the text before it kept, as the
/// packager puts CurseForge's export there. A locale without a file, and a
/// single-key line (`key=`), are left as written.
pub fn localize(text: &str, file_of: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches('\n');
        let content = body.find("--@localization(").and_then(|start| {
            let params = &body[start + "--@localization(".len()..];
            let params = &params[..params.find(")@")?];
            if params.contains("key=") {
                return None;
            }
            let locale = params.split("locale=\"").nth(1)?.split('"').next()?;
            if locale.is_empty() || !locale.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            Some(format!(
                "{}{}",
                &body[..start],
                file_of(locale)?.trim_end_matches('\n')
            ))
        });
        match content {
            Some(c) => {
                out.push_str(&c);
                out.push_str(&line[body.len()..]);
            }
            None => out.push_str(line),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localization_reads_local_files() {
        let lua = "local L = {}\n  --@localization(locale=\"deDE\", format=\"lua_additive_table\")@\n--@localization(locale=\"frFR\")@\nL.x = --@localization(locale=\"deDE\", key=\"x\")@\nT = --@localization(locale=\"deDE\", format=\"lua_table\")@\n";
        let de =
            |l: &str| (l == "deDE").then(|| "L[\"a\"] = \"b\"\nL[\"c\"] = \"d\"\n".to_string());
        assert_eq!(
            localize(lua, de),
            "local L = {}\n  L[\"a\"] = \"b\"\nL[\"c\"] = \"d\"\n--@localization(locale=\"frFR\")@\nL.x = --@localization(locale=\"deDE\", key=\"x\")@\nT = L[\"a\"] = \"b\"\nL[\"c\"] = \"d\"\n"
        );
        assert_eq!(
            localize("--@localization(locale=\"../x\")@", |_| Some("no".into())),
            "--@localization(locale=\"../x\")@"
        );
    }

    #[test]
    fn blocks_follow_the_packager() {
        let lua = "--@debug@\nprint(1)\n--@end-debug@\n--[===[@non-debug@\nprint(2)\n--@end-non-debug@]===]\n";
        let info = KeywordInfo {
            version: "1.2.3".into(),
            ..Default::default()
        };
        assert_eq!(
            apply(lua, &info, Path::new("a.lua"), Path::new("a.lua"), false),
            "--[==[@debug@\nprint(1)\n--@end-debug@]==]\n--@non-debug@\nprint(2)\n--@end-non-debug@\n"
        );
        let on = KeywordInfo {
            debug: true,
            ..info.clone()
        };
        assert_eq!(
            apply(lua, &on, Path::new("a.lua"), Path::new("a.lua"), false),
            lua
        );
        assert_eq!(
            apply(
                "v = \"@project-version@\"",
                &info,
                Path::new("a.lua"),
                Path::new("a.lua"),
                false
            ),
            "v = \"1.2.3\""
        );
        let xml = "<!--@debug@-->\n<Script file=\"x.lua\"/>\n<!--@end-debug@-->\n";
        assert_eq!(
            apply(xml, &info, Path::new("a.xml"), Path::new("a.xml"), true),
            "<!--@debug@\n<Script file=\"x.lua\"/>\n@end-debug@-->\n"
        );
        let flavored = "--@version-classic@\nx()\n--@end-version-classic@\n";
        assert_eq!(apply_flavor(flavored, "vanilla", false), flavored);
        assert_eq!(
            apply_flavor(flavored, "standard", false),
            "--[====[@version-classic@\nx()\n--@end-version-classic@]====]\n"
        );
        let dnp = "a\n--@do-not-package@\ntest()\n--@end-do-not-package@\nb\n";
        assert_eq!(
            apply(dnp, &info, Path::new("a.lua"), Path::new("a.lua"), false),
            "a\nb\n"
        );
        assert_eq!(
            apply(dnp, &on, Path::new("a.lua"), Path::new("a.lua"), false),
            dnp
        );
        let libxml = "<Ui>\n<!--@no-lib-strip@-->\n<Script file=\"LibStub.lua\"/>\n<!--@end-no-lib-strip@-->\n</Ui>\n";
        assert_eq!(
            apply_library(libxml, &info, true),
            "<Ui>\n<!--@no-lib-strip@\n<Script file=\"LibStub.lua\"/>\n@end-no-lib-strip@-->\n</Ui>\n"
        );
        let toc = "A.lua\n#@non-debug@\n# Libs\\X.lua\n#@end-non-debug@\n#@do-not-package@\nDev.lua\n#@end-do-not-package@\n#@debug@\nDbg.lua\n#@end-debug@\n#@no-lib-strip@\nL.lua\n#@end-no-lib-strip@\n";
        assert_eq!(
            packaged_toc(toc),
            "A.lua\nLibs\\X.lua\n#@no-lib-strip@\nL.lua\n#@end-no-lib-strip@\n"
        );
        assert_eq!(dates(0).0, "1970-01-01T00:00:00Z");
        assert_eq!(dates(1_760_243_127).1, "20251012042527");
    }

    #[test]
    fn packaged_toc_for_a_game() {
        let toc = "## Interface: 1\n#@retail@\nretail.lua\n#@end-retail@\n#@non-retail@\n# classic.lua\n#@end-non-retail@\n";
        assert_eq!(
            packaged_toc_for(toc, Some("standard")),
            "## Interface: 1\nretail.lua\n"
        );
        assert_eq!(
            packaged_toc_for(toc, Some("vanilla")),
            "## Interface: 1\nclassic.lua\n"
        );
        // Without a game, flavor blocks stay as they are.
        assert!(packaged_toc(toc).contains("#@retail@"));
    }

    #[test]
    fn toc_version_placeholders_take_current_numbers() {
        let text =
            toc_version_placeholders("## Interface: @toc-version-retail@, @toc-version-classic@\n");
        let table = wowlua_flavors::table();
        let n = |id: &str| table.flavor(id).unwrap().interface[0];
        assert_eq!(
            text,
            format!("## Interface: {}, {}\n", n("standard"), n("vanilla"))
        );
    }
}
