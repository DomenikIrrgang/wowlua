//! BigWigs packager keyword substitution (spec CLI §12.6, `compat.packager-
//! keywords`): a text preprocessing step for passthrough `.lua` and `.xml`
//! files. Repository keywords are replaced; inactive blocks are commented
//! out in the packager's style, so line numbers stay the same.

use std::path::Path;
use std::process::{Command, Stdio};

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
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
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
        let mut info = KeywordInfo {
            version: version.to_string(),
            debug,
            alpha,
            build_timestamp: now,
            ..Default::default()
        };
        if let Some(line) = git(
            dir,
            &["log", "-1", "--abbrev=7", "--format=%H%x09%h%x09%an%x09%ct"],
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
    ("version-titan", "wrath"),
    ("version-cata", "cata"),
    ("version-mists", "mists"),
    ("version-forever", "camelot"),
];

/// Whether `text` uses flavor blocks.
pub fn has_flavor_blocks(text: &str) -> bool {
    FLAVOR_KEYWORDS
        .iter()
        .any(|(k, _)| text.contains(&format!("@{k}@")))
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

fn substitute(text: &str, info: &KeywordInfo) -> String {
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
pub fn apply(text: &str, info: &KeywordInfo, rel: &Path, xml: bool) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    let mut out = substitute(text, info);
    if out.contains("@file-")
        && let Some(repo) = &info.repo
    {
        let path = rel.to_string_lossy().into_owned();
        let line = git(
            repo,
            &[
                "log",
                "-1",
                "--abbrev=7",
                "--format=%H%x09%h%x09%an%x09%ct",
                "--",
                &path,
            ],
        )
        .unwrap_or_default();
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 4 {
            let ts: i64 = parts[3].parse().unwrap_or(0);
            let (iso, int) = dates(ts);
            let count =
                git(repo, &["rev-list", "--count", "HEAD", "--", &path]).unwrap_or_default();
            out = out
                .replace("@file-revision@", &count)
                .replace("@file-hash@", parts[0])
                .replace("@file-abbreviated-hash@", parts[1])
                .replace("@file-author@", parts[2])
                .replace("@file-date-iso@", &iso)
                .replace("@file-date-integer@", &int)
                .replace("@file-timestamp@", &ts.to_string());
        }
    }
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
    let mut out = String::with_capacity(text.len());
    // Which block a line is in: (keyword, negated).
    let mut block: Option<(&str, bool)> = None;
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
        for kw in ["do-not-package", "debug", "alpha"] {
            if let Some((start, neg)) = marker(kw) {
                block = if start { Some((kw, neg)) } else { None };
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        match block {
            // Inactive blocks are dropped.
            Some((_, false)) => {}
            // `non-` blocks are uncommented.
            Some((_, true)) => {
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

/// `@localization(…)@` is not supported (W4030).
pub fn uses_localization(text: &str) -> bool {
    text.contains("@localization(")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_follow_the_packager() {
        let lua = "--@debug@\nprint(1)\n--@end-debug@\n--[===[@non-debug@\nprint(2)\n--@end-non-debug@]===]\n";
        let info = KeywordInfo {
            version: "1.2.3".into(),
            ..Default::default()
        };
        assert_eq!(
            apply(lua, &info, Path::new("a.lua"), false),
            "--[==[@debug@\nprint(1)\n--@end-debug@]==]\n--@non-debug@\nprint(2)\n--@end-non-debug@\n"
        );
        let on = KeywordInfo {
            debug: true,
            ..info.clone()
        };
        assert_eq!(apply(lua, &on, Path::new("a.lua"), false), lua);
        assert_eq!(
            apply(
                "v = \"@project-version@\"",
                &info,
                Path::new("a.lua"),
                false
            ),
            "v = \"1.2.3\""
        );
        let xml = "<!--@debug@-->\n<Script file=\"x.lua\"/>\n<!--@end-debug@-->\n";
        assert_eq!(
            apply(xml, &info, Path::new("a.xml"), true),
            "<!--@debug@\n<Script file=\"x.lua\"/>\n@end-debug@-->\n"
        );
        let flavored = "--@version-classic@\nx()\n--@end-version-classic@\n";
        assert_eq!(apply_flavor(flavored, "vanilla", false), flavored);
        assert_eq!(
            apply_flavor(flavored, "standard", false),
            "--[====[@version-classic@\nx()\n--@end-version-classic@]====]\n"
        );
        let dnp = "a\n--@do-not-package@\ntest()\n--@end-do-not-package@\nb\n";
        assert_eq!(apply(dnp, &info, Path::new("a.lua"), false), "a\nb\n");
        assert_eq!(apply(dnp, &on, Path::new("a.lua"), false), dnp);
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
}
