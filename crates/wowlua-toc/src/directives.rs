//! The TOC directives wowlua knows (spec §9.3, §14.9): those it writes from
//! `wowlua.json` and those an author may add through `addon.toc-extra`.
//! Plain data, for the manifest's checks and for editors (Studio's picker
//! of `toc-extra` keys).

/// A TOC directive (`## Name: value`) and what it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownDirective {
    /// The directive's name, as the client spells it.
    pub name: &'static str,
    /// What it does, in one line.
    pub meaning: &'static str,
    /// The `wowlua.json` key wowlua writes it from (generated directives
    /// only).
    pub from: Option<&'static str>,
}

const fn generated(
    name: &'static str,
    from: &'static str,
    meaning: &'static str,
) -> KnownDirective {
    KnownDirective {
        name,
        meaning,
        from: Some(from),
    }
}

const fn extra(name: &'static str, meaning: &'static str) -> KnownDirective {
    KnownDirective {
        name,
        meaning,
        from: None,
    }
}

/// The directives wowlua writes itself from other manifest keys. In
/// `addon.toc-extra` (or `toc-data`) they are error E4002. `Title`, `Notes`
/// and `Category` also cover their localized forms (`Title-deDE`).
pub const GENERATED_DIRECTIVES: &[KnownDirective] = &[
    generated(
        "Interface",
        "flavors.<flavor>.interface",
        "The client versions the addon is up to date for.",
    ),
    generated(
        "Title",
        "addon.title",
        "The addon's name in the addon list.",
    ),
    generated("Notes", "addon.notes", "The addon list's tooltip text."),
    generated("Author", "addon.author", "Who made the addon."),
    generated("Version", "package.version", "The addon's version."),
    generated(
        "Category",
        "addon.category",
        "The addon list's category the addon is shown under.",
    ),
    generated(
        "Group",
        "addon.group",
        "The addon whose entry this one is grouped under in the addon list.",
    ),
    generated(
        "IconTexture",
        "addon.icon-texture",
        "The addon list's icon, a texture.",
    ),
    generated(
        "IconAtlas",
        "addon.icon-atlas",
        "The addon list's icon, an atlas.",
    ),
    generated(
        "LoadOnDemand",
        "addon.load-on-demand",
        "Loaded only when code asks for it (C_AddOns.LoadAddOn).",
    ),
    generated(
        "DefaultState",
        "addon.default-state",
        "Whether a newly installed addon starts enabled.",
    ),
    generated(
        "SavedVariables",
        "addon.saved-variables",
        "Globals the client saves for the account.",
    ),
    generated(
        "SavedVariablesPerCharacter",
        "addon.saved-variables-per-character",
        "Globals the client saves per character.",
    ),
    generated(
        "LoadSavedVariablesFirst",
        "addon.load-saved-variables-first",
        "SavedVariables load before the addon's files.",
    ),
    generated(
        "AllowAddOnTableAccess",
        "addon.allow-table-access",
        "Other addons may read the addon's namespace table.",
    ),
    generated(
        "OnlyBetaAndPTR",
        "addon.only-beta-and-ptr",
        "Loads only on beta and PTR clients.",
    ),
    generated(
        "Dependencies",
        "addon-dependencies.required",
        "Addons that must load first.",
    ),
    generated(
        "RequiredDeps",
        "addon-dependencies.required",
        "Addons that must load first (another name).",
    ),
    generated(
        "OptionalDeps",
        "addon-dependencies.optional",
        "Addons that load first when installed.",
    ),
    generated(
        "LoadWith",
        "addon-dependencies.load-with",
        "Load-on-demand: loads together with these addons.",
    ),
    generated(
        "LoadManagers",
        "addon-dependencies.load-managers",
        "Load-on-demand only when one of these addons is installed.",
    ),
    generated(
        "X-Wowlua-Build",
        "the build's profile",
        "The profile and build ID of a wowlua build.",
    ),
];

/// The directives an author may set through `addon.toc-extra`: the client's
/// that wowlua does not write, and common `X-` fields (any `X-` name is
/// allowed: custom metadata, read with `C_AddOns.GetAddOnMetadata`).
///
/// `X-Curse-Project-ID`, `X-Wago-ID` and `X-WoWI-ID` are not listed: wowlua
/// writes them from `publish.targets.<name>.project-id`.
pub const EXTRA_DIRECTIVES: &[KnownDirective] = &[
    extra(
        "AddonCompartmentFunc",
        "The global function the minimap's addon compartment calls on a click.",
    ),
    extra(
        "AddonCompartmentFuncOnEnter",
        "The global function the addon compartment calls when the mouse enters the entry.",
    ),
    extra(
        "AddonCompartmentFuncOnLeave",
        "The global function the addon compartment calls when the mouse leaves the entry.",
    ),
    extra(
        "AllowLoadGameType",
        "Game types the whole addon loads on (mainline, classic, or a flavor).",
    ),
    extra(
        "ExcludeLoadGameType",
        "Game types the whole addon does not load on.",
    ),
    extra(
        "X-Website",
        "The addon's website (metadata for addons and sites).",
    ),
    extra("X-Email", "The author's e-mail address (metadata)."),
    extra("X-License", "The addon's license (metadata)."),
    extra("X-Credits", "Who else helped (metadata)."),
    extra(
        "X-Localizations",
        "The locales the addon is translated into (metadata).",
    ),
];

/// The directives an author may add in `addon.toc-extra`, for a picker:
/// the known ones wowlua does not generate (any other `X-` name is allowed
/// too).
pub fn extra_directives() -> &'static [KnownDirective] {
    EXTRA_DIRECTIVES
}

/// The directives wowlua writes from `wowlua.json`.
pub fn generated_directives() -> &'static [KnownDirective] {
    GENERATED_DIRECTIVES
}

/// Whether wowlua generates this TOC directive itself (so it must not come
/// from `toc-extra` or `toc-data`), ignoring case. Localized variants count
/// for the localizable directives only (`Title-deDE`, not
/// `IconTexture-blank`).
pub fn is_generated_directive(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    GENERATED_DIRECTIVES
        .iter()
        .any(|d| d.name.eq_ignore_ascii_case(key))
        || ["title-", "notes-", "category-"]
            .iter()
            .any(|p| lower.starts_with(p))
}

/// Whether `key` may stand in `toc-extra` without warning W4001: a known
/// directive wowlua does not generate, or a custom `X-` field.
pub fn is_known_extra_directive(key: &str) -> bool {
    key.starts_with("X-")
        || EXTRA_DIRECTIVES
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_directives_are_not_generated() {
        for d in extra_directives() {
            assert!(!is_generated_directive(d.name), "{}", d.name);
            assert!(is_known_extra_directive(d.name), "{}", d.name);
            assert!(d.from.is_none() && !d.meaning.is_empty(), "{}", d.name);
        }
        for d in generated_directives() {
            assert!(is_generated_directive(d.name), "{}", d.name);
            assert!(d.from.is_some(), "{}", d.name);
        }
        assert!(is_generated_directive("title-deDE"));
        assert!(!is_generated_directive("IconTexture-blank"));
        assert!(is_known_extra_directive("X-Discord"));
        assert!(!is_known_extra_directive("Website"));
    }
}
