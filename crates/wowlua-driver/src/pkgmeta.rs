//! `.pkgmeta` (the BigWigs packager's project file): reading it, and
//! converting its externals into dependencies (spec CLI §12.6, step 6).
//! External URLs are interpreted like the packager does: old CurseForge
//! URLs, SVN `trunk` / `tags` paths, sub-paths of CurseForge and GitHub git
//! URLs, `.zip` archives, and git otherwise.

use serde_json::{Map, Value, json};
use std::path::Path;
use wowlua_diagnostics::Diagnostic;
use yaml_rust2::{Yaml, YamlLoader};

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
    /// `tag: latest`, which wowlua cannot pin.
    pub latest: bool,
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

fn scalar(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) | Yaml::Real(s) => Some(s.trim().to_string()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn list(y: &Yaml) -> Vec<String> {
    match y {
        Yaml::Array(items) => items.iter().filter_map(scalar).collect(),
        other => scalar(other).into_iter().collect(),
    }
}

fn truthy(y: &Yaml) -> bool {
    scalar(y)
        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "on" | "1"))
}

pub fn parse(text: &str) -> Result<PkgMeta, String> {
    let docs = YamlLoader::load_from_str(text).map_err(|e| e.to_string())?;
    let Some(root) = docs.into_iter().next() else {
        return Ok(PkgMeta::default());
    };
    let mut meta = PkgMeta::default();
    let Yaml::Hash(map) = &root else {
        return Ok(meta);
    };
    for (key, value) in map {
        let Some(key) = scalar(key) else { continue };
        match key.as_str() {
            "package-as" => meta.package_as = scalar(value),
            "ignore" => meta.ignore = list(value),
            "plain-copy" => meta.plain_copy = list(value),
            "enable-nolib-creation" => meta.enable_nolib = truthy(value),
            "manual-changelog" => {
                meta.manual_changelog = match value {
                    Yaml::Hash(h) => h
                        .get(&Yaml::String("filename".into()))
                        .and_then(scalar)
                        .map(|f| {
                            (
                                f,
                                h.get(&Yaml::String("markup-type".into())).and_then(scalar),
                            )
                        }),
                    other => scalar(other).map(|f| (f, None)),
                }
            }
            "move-folders" => {
                if let Yaml::Hash(h) = value {
                    for (from, to) in h {
                        if let (Some(f), Some(t)) = (scalar(from), scalar(to)) {
                            meta.move_folders.push((f, t));
                        }
                    }
                }
            }
            "required-dependencies"
            | "optional-dependencies"
            | "embedded-libraries"
            | "tools-used" => {
                let kind = match key.as_str() {
                    "required-dependencies" => "requiredDependency",
                    "optional-dependencies" => "optionalDependency",
                    "embedded-libraries" => "embeddedLibrary",
                    _ => "tool",
                };
                for slug in list(value) {
                    meta.relations.push((slug, kind.to_string()));
                }
            }
            "externals" => {
                if let Yaml::Hash(h) = value {
                    for (dir, spec) in h {
                        let Some(dir) = scalar(dir) else { continue };
                        if let Some(e) = external(&dir, spec) {
                            meta.externals.push(e);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(meta)
}

/// One external, interpreted like the packager's `process_external`.
fn external(dir: &str, spec: &Yaml) -> Option<External> {
    let get = |k: &str| match spec {
        Yaml::Hash(h) => h.get(&Yaml::String(k.into())).and_then(scalar),
        _ => None,
    };
    let mut uri = match spec {
        Yaml::Hash(_) => get("url")?,
        other => scalar(other)?,
    };
    // Trailing comments, spaces and slashes.
    if let Some(i) = uri.find('#') {
        uri.truncate(i);
    }
    let mut uri = uri.trim().trim_end_matches('/').to_string();
    let mut tag = get("tag");
    let branch = get("branch");
    let commit = get("commit");
    let mut kind = get("type");
    let mut slug = get("curse-slug");
    let mut path = get("path");

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
    if latest {
        tag = None;
    }
    let source = match kind.as_str() {
        "svn" => {
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
}
