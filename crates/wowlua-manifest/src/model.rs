//! The typed model of `wowlua.json` (spec chapter 9).
//!
//! Every struct rejects unknown keys (E4002). Keys are kebab-case. Defaults
//! follow the spec tables.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

pub type JsonObject = Map<String, Value>;

fn yes() -> bool {
    true
}

/// Deserializes a field that distinguishes "absent" (outer `None`, via
/// `#[serde(default)]`) from an explicit `null` (`Some(None)`).
fn explicit_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
#[schemars(title = "wowlua.json", description = "Manifest of a wowlua package")]
pub struct Manifest {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Required, except in a workspace root manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<Package>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addon: Option<Addon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lua_library: Option<LuaLibrarySection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<TypesSection>,
    /// Supported flavors: flavor IDs or aliases mapped to their settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavors: Option<IndexMap<String, FlavorSettings>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, Dependency>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub dev_dependencies: IndexMap<String, Dependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addon_dependencies: Option<AddonDependencies>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub features: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub build: Build,
    #[serde(default)]
    pub check: Check,
    /// User configuration keys with their default values (§9.9.1).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub config: JsonObject,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub profiles: IndexMap<String, ProfileSettings>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub minify_levels: IndexMap<String, MinifyLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<Publish>,
    #[serde(default)]
    pub meta: MetaSettings,
    #[serde(default)]
    pub fmt: Fmt,
    #[serde(default)]
    pub compat: Compat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    #[serde(default)]
    pub stdlib: Stdlib,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PackageKind {
    #[default]
    Addon,
    Library,
    LuaLibrary,
    Types,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Package {
    /// Package ID: lowercase `[a-z0-9-]`, 2–64 characters.
    pub name: String,
    /// SemVer 2.0 version.
    pub version: String,
    #[serde(default)]
    pub kind: PackageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// SPDX license expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// Required compiler version (semver requirement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wowlua: Option<String>,
    /// Main module of a library (default `src/init.wowlua`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultState {
    #[default]
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MetaValue {
    Bool(bool),
    Number(serde_json::Number),
    String(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Addon {
    /// AddOn folder and TOC name. Default: PascalCase of `package.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub title_localized: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub notes_localized: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub category_localized: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_texture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_atlas: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub load_on_demand: bool,
    #[serde(default)]
    pub default_state: DefaultState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_variables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_variables_per_character: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub load_saved_variables_first: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_table_access: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub only_beta_and_ptr: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose_meta: Option<String>,
    /// Extra TOC directives, emitted verbatim.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub toc_extra: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub meta: IndexMap<String, MetaValue>,
    /// Metadata that differs per flavor: flavor → fields. `null` removes a
    /// field for that flavor (§9.3.1).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub flavor_overrides: IndexMap<String, AddonOverride>,
    /// Files of verbatim `## Key: Value` lines appended to a flavor's TOC, for
    /// bulk or generated metadata (§9.3.2): flavor → files.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub toc_data: IndexMap<String, Vec<String>>,
}

/// Per-flavor overrides of addon metadata. An absent field keeps the base
/// value; `null` removes the field for the flavor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AddonOverride {
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub title_localized: IndexMap<String, Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub notes: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub notes_localized: IndexMap<String, Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub author: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub category: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub category_localized: IndexMap<String, Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub group: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon_texture: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon_atlas: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub toc_extra: IndexMap<String, Option<String>>,
    /// This flavor's own SavedVariables (decision T128).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_variables: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_variables_per_character: Option<Vec<String>>,
}

impl Addon {
    /// Every SavedVariables name of any flavor.
    pub fn all_saved_variables(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let lists = std::iter::once(&self.saved_variables)
            .chain(std::iter::once(&self.saved_variables_per_character))
            .chain(self.flavor_overrides.values().flat_map(|o| {
                o.saved_variables
                    .iter()
                    .chain(o.saved_variables_per_character.iter())
            }));
        for list in lists {
            for v in list {
                if !out.contains(v) {
                    out.push(v.clone());
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LuaLibrarySection {
    /// Ordered entry files (`.lua`, `.xml` or a `.toc`).
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TypesSection {
    #[serde(default)]
    pub declarations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FlavorSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_folders: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
}

/// A dependency: a version requirement string or a source object.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Dependency {
    Version(String),
    Detailed(Box<DependencyDetail>),
}

impl<'de> Deserialize<'de> for Dependency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(req) => Ok(Dependency::Version(req)),
            value @ Value::Object(_) => DependencyDetail::deserialize(value)
                .map(|d| Dependency::Detailed(Box::new(d)))
                .map_err(D::Error::custom),
            other => Err(D::Error::custom(format!(
                "expected a version requirement string or a dependency object, found {}",
                json_type_name(&other)
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct DependencyDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub submodules: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip_prefix: Option<String>,
    /// The trailing folders of `subdir` / `strip-prefix` a `.pkgmeta`
    /// `move-folders` moved up: the files of the folder above stay next to
    /// them in the output, like the packager leaves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_up: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PackageKind>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flavors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addon_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(default = "yes")]
    pub default_features: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub config: JsonObject,
}

impl Dependency {
    pub fn is_optional(&self) -> bool {
        matches!(self, Dependency::Detailed(d) if d.optional)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AddonDependency {
    Name(String),
    Detailed(AddonDependencyDetail),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AddonDependencyDetail {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flavors: Vec<String>,
}

impl AddonDependency {
    pub fn name(&self) -> &str {
        match self {
            AddonDependency::Name(name) => name,
            AddonDependency::Detailed(d) => &d.name,
        }
    }

    pub fn flavors(&self) -> &[String] {
        match self {
            AddonDependency::Name(_) => &[],
            AddonDependency::Detailed(d) => &d.flavors,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct AddonDependencies {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<AddonDependency>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<AddonDependency>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_managers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FlavorMode {
    #[default]
    Split,
    Shared,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TocLayout {
    #[default]
    Auto,
    Single,
    PerFlavor,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LocaleLoading {
    #[default]
    Client,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Build {
    #[serde(default = "Build::default_src")]
    pub src: String,
    #[serde(default = "Build::default_out")]
    pub out: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_order: Vec<String>,
    /// Flavors with an order of their own (flavor → like `load-order`),
    /// written by `wowlua init` when the flavors' TOCs disagree.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub flavor_load_order: IndexMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep: Vec<String>,
    #[serde(default = "Build::default_libs_dir")]
    pub libs_dir: String,
    /// Restricts matching source files to the listed flavors (glob → flavors).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub flavor_files: IndexMap<String, Vec<String>>,
    /// TOC file lines with variables (`[Game]`, `[Family]`, `[TextLocale]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable_files: Vec<VariableFile>,
    /// TOC file lines loading files of another AddOn (`../<AddOn>/<path>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_files: Vec<ExternalFile>,
    /// Sources that are built and shipped but loaded by another AddOn's TOC
    /// instead of this one's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_files: Vec<String>,
    #[serde(default)]
    pub flavor_mode: FlavorMode,
    #[serde(default)]
    pub toc_layout: TocLayout,
    #[serde(default)]
    pub locales: LocaleLoading,
    #[serde(default = "Build::default_locales_dir")]
    pub locales_dir: String,
    #[serde(default = "Build::default_locale_base")]
    pub locale_base: String,
}

impl Build {
    fn default_src() -> String {
        "src".into()
    }
    fn default_out() -> String {
        "dist".into()
    }
    fn default_libs_dir() -> String {
        "libs".into()
    }
    fn default_locales_dir() -> String {
        "locales".into()
    }
    fn default_locale_base() -> String {
        "enUS".into()
    }
}

impl Default for Build {
    fn default() -> Self {
        Build {
            variable_files: Vec::new(),
            external_files: Vec::new(),
            shared_files: Vec::new(),
            src: Self::default_src(),
            out: Self::default_out(),
            assets: Vec::new(),
            exclude: Vec::new(),
            load_order: Vec::new(),
            flavor_load_order: IndexMap::new(),
            keep: Vec::new(),
            libs_dir: Self::default_libs_dir(),
            flavor_files: IndexMap::new(),
            flavor_mode: FlavorMode::default(),
            toc_layout: TocLayout::default(),
            locales: LocaleLoading::default(),
            locales_dir: Self::default_locales_dir(),
            locale_base: Self::default_locale_base(),
        }
    }
}

/// A TOC file line with variables (§9.7.1).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct VariableFile {
    /// Path relative to the package root, e.g. `src/Locales/[TextLocale].lua`.
    pub path: String,
    /// `[TextLocale]` only: the locales this line loads for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<String>>,
    /// Restricts the line to these flavors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavors: Option<Vec<String>>,
}

/// A TOC file line loading a file of another AddOn (§9.7.2).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ExternalFile {
    /// `../<AddOn>/<path>`: the file's path in that AddOn's output, relative
    /// to this AddOn's folder.
    pub path: String,
    /// Restricts the line to these flavors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavors: Option<Vec<String>>,
}

/// The variables a TOC file line may contain.
pub const TOC_VARIABLES: &[&str] = &["[Game]", "[Family]", "[TextLocale]"];

/// Client locale codes, as `GetLocale()` reports them.
pub const LOCALES: &[&str] = &[
    "enUS", "enGB", "deDE", "esES", "esMX", "frFR", "itIT", "koKR", "ptBR", "ruRU", "zhCN", "zhTW",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CheckMode {
    Nocheck,
    #[default]
    Basic,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LintSetting {
    Allow,
    Warn,
    Deny,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Check {
    #[serde(default)]
    pub mode: CheckMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strict: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lua: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub globals: Vec<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub lints: IndexMap<String, LintSetting>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Instrument {
    None,
    Decorated,
    Exports,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeChecks {
    None,
    Exports,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Reflection {
    None,
    Decorated,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LineMode {
    Preserve,
    Statement,
    Pretty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CacheGlobals {
    Off,
    Safe,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct OptimizeSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_globals: Option<CacheGlobals>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_code: Option<bool>,
}

/// Settings of one profile as written in the manifest. Unset values are
/// inherited (§9.9).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ProfileSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assertions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<LogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<Instrument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_checks: Option<RuntimeChecks>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_errors: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<Reflection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_console: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_errors: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_maps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minify: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minify_libraries: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize: Option<OptimizeSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_mode: Option<LineMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<JsonObject>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct MinifyLevel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(default)]
    pub passes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChangelogFormat {
    Markdown,
    Text,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseType {
    Auto,
    Release,
    Beta,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BundleAddons {
    All(bool),
    Listed(Vec<String>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Publish {
    /// Path, `"git"`, or `null` for no changelog.
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub changelog: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog_format: Option<ChangelogFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_type: Option<ReleaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_maps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_addons: Option<BundleAddons>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub targets: IndexMap<String, Target>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TargetType {
    Curseforge,
    Wago,
    Wowinterface,
    Github,
    Registry,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ProjectId {
    Number(u64),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Relation {
    pub slug: String,
    #[serde(rename = "type")]
    pub relation_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChangelogOutput {
    Bbcode,
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Prerelease {
    Bool(bool),
    Auto(AutoKeyword),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AutoKeyword {
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Target {
    #[serde(rename = "type")]
    pub target_type: TargetType,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minify: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub split_flavors: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nolib: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_type: Option<ReleaseType>,
    #[serde(
        default,
        deserialize_with = "explicit_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub changelog: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog_format: Option<ChangelogFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog_output: Option<ChangelogOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relations: Option<Vec<Relation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_previous: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_untagged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_json: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerelease: Option<Prerelease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MetaRuntime {
    #[default]
    Used,
    Full,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct MetaSettings {
    #[serde(default)]
    pub runtime: MetaRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Indent {
    Tab(TabKeyword),
    Spaces(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TabKeyword {
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QuoteStyle {
    Double,
    Single,
    Preserve,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Fmt {
    #[serde(default = "Fmt::default_indent")]
    pub indent: Indent,
    #[serde(default = "Fmt::default_line_width")]
    pub line_width: u32,
    #[serde(default = "Fmt::default_quote_style")]
    pub quote_style: QuoteStyle,
    #[serde(default)]
    pub lua: bool,
}

impl Fmt {
    fn default_indent() -> Indent {
        Indent::Tab(TabKeyword::Tab)
    }
    fn default_line_width() -> u32 {
        100
    }
    fn default_quote_style() -> QuoteStyle {
        QuoteStyle::Double
    }
}

impl Default for Fmt {
    fn default() -> Self {
        Fmt {
            indent: Self::default_indent(),
            line_width: Self::default_line_width(),
            quote_style: Self::default_quote_style(),
            lua: false,
        }
    }
}

/// Line endings of packager-processed files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LineEndings {
    Lf,
    Crlf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Compat {
    #[serde(default)]
    pub pkgmeta: bool,
    #[serde(default)]
    pub packager_keywords: bool,
    /// Files the packager copies unprocessed (`.pkgmeta` `plain-copy`):
    /// shell patterns relative to the source folder or the package root,
    /// `*` also matching `/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plain_copy: Vec<String>,
    /// The AddOn the packager's package folder becomes in a workspace
    /// without a root addon (`.pkgmeta` `package-as`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_as: Option<String>,
    /// The TOC `## Version` value as the packager writes it: the original
    /// TOC's value, keywords substituted (`""`: no `Version` line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toc_version: Option<String>,
    /// Line endings of the files the packager processes: `crlf` (its
    /// default) or `lf` (`-u`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_endings: Option<LineEndings>,
    /// The title of the changelog the packager generates (`.pkgmeta`
    /// `changelog-title`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog_title: Option<String>,
    /// The project is released one game per packager run (`-g`): packager
    /// flavor blocks apply per flavor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub per_game: bool,
    /// The license file (`.pkgmeta` `license-output`) the workspace root
    /// copies into each member's output that has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_output: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Workspace {
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_members: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// AddOns that are fetched and shipped as they are (a `.pkgmeta`
    /// external that `move-folders` makes an AddOn): AddOn folder name →
    /// source (`git`, `svn`, `archive` or `path`, with `subdir` /
    /// `strip-prefix`).
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub external_addons: IndexMap<String, DependencyDetail>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Stdlib {
    #[serde(default)]
    pub log: StdlibLog,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct StdlibLog {
    #[serde(default = "StdlibLog::default_history_size")]
    pub history_size: u32,
}

impl StdlibLog {
    fn default_history_size() -> u32 {
        500
    }
}

impl Default for StdlibLog {
    fn default() -> Self {
        StdlibLog {
            history_size: Self::default_history_size(),
        }
    }
}

pub(crate) fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}
