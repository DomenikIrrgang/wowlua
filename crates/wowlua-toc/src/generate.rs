//! TOC generation (spec §14.9): layout selection, per-flavor file names,
//! inline load directives and the 1024-character line rule.

use crate::{APPENDABLE_DIRECTIVES, MAX_LINE_LENGTH};
use wowlua_flavors::{Client, Family, Flavor, FlavorTable};

/// The requested TOC layout (`build.toc-layout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Auto,
    Single,
    PerFlavor,
}

/// A targeted flavor and the interface numbers to emit for it.
#[derive(Clone, Debug)]
pub struct FlavorTarget {
    pub id: String,
    pub interfaces: Vec<u32>,
}

/// A metadata directive (`## Key: Value`), optionally limited to flavors.
#[derive(Clone, Debug)]
pub struct Directive {
    pub key: String,
    pub value: String,
    /// `None` means every targeted flavor.
    pub flavors: Option<Vec<String>>,
}

impl Directive {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Directive {
            key: key.into(),
            value: value.into(),
            flavors: None,
        }
    }
}

/// A file line. `path` uses `/` separators; it is written with `\`.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    /// `None` means every targeted flavor.
    pub flavors: Option<Vec<String>>,
    /// `Some(code)`: load only for that client locale.
    pub locale: Option<String>,
}

impl FileEntry {
    pub fn new(path: impl Into<String>) -> Self {
        FileEntry {
            path: path.into(),
            flavors: None,
            locale: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TocInput {
    pub addon_name: String,
    pub flavors: Vec<FlavorTarget>,
    /// Metadata directives in output order (without `Interface`).
    pub directives: Vec<Directive>,
    /// Files in load order.
    pub files: Vec<FileEntry>,
    pub layout: Layout,
}

/// One generated TOC file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocFile {
    /// File name, e.g. `MyAddon.toc` or `MyAddon_Classic.toc`.
    pub name: String,
    /// Flavors this file serves.
    pub flavors: Vec<String>,
    pub content: String,
}

/// The generated TOC files and the layout actually used.
#[derive(Clone, Debug)]
pub struct Generated {
    pub layout: Layout,
    pub files: Vec<TocFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TocError {
    /// `toc-layout = "single"` is forced, but a targeted client lacks a needed
    /// TOC feature (E4301).
    SingleUnsupported { client: String, feature: String },
    /// Per-flavor TOC names cannot tell these flavors apart (E4302).
    Indistinguishable { flavors: Vec<String> },
    /// A line longer than the client reads that cannot be split (E4303).
    LineTooLong { toc: String, line: String },
}

/// Generates the TOC file(s) for one build.
pub fn generate(input: &TocInput) -> Result<Generated, Vec<TocError>> {
    let table = wowlua_flavors::table();
    let input = normalized(input);
    let single_problem = single_layout_problem(table, &input);
    let layout = match (input.layout, single_problem) {
        (Layout::Single, Some((client, feature))) => {
            return Err(vec![TocError::SingleUnsupported { client, feature }]);
        }
        (Layout::Single, None) | (Layout::Auto, None) => Layout::Single,
        (Layout::PerFlavor, _) | (Layout::Auto, Some(_)) => Layout::PerFlavor,
    };
    let layout = if input.flavors.len() == 1 {
        // With one flavor both layouts produce the same plain `<Name>.toc`.
        Layout::Single
    } else {
        layout
    };

    let files = match layout {
        Layout::Single => vec![render_single(table, &input)],
        _ => render_per_flavor(table, &input)?,
    };
    let errors: Vec<TocError> = files.iter().flat_map(check_lines).collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Generated { layout, files })
}

/// Drops flavor restrictions that cover every targeted flavor.
fn normalized(input: &TocInput) -> TocInput {
    let all: Vec<&str> = input.flavors.iter().map(|f| f.id.as_str()).collect();
    let simplify = |flavors: &Option<Vec<String>>| {
        flavors.as_ref().and_then(|list| {
            let covers_all = all.iter().all(|id| list.iter().any(|f| f == id));
            (!covers_all).then(|| {
                all.iter()
                    .filter(|id| list.iter().any(|f| f == *id))
                    .map(|id| id.to_string())
                    .collect()
            })
        })
    };
    TocInput {
        addon_name: input.addon_name.clone(),
        flavors: input.flavors.clone(),
        directives: input
            .directives
            .iter()
            .map(|d| Directive {
                flavors: simplify(&d.flavors),
                ..d.clone()
            })
            .collect(),
        files: input
            .files
            .iter()
            .map(|f| FileEntry {
                flavors: simplify(&f.flavors),
                ..f.clone()
            })
            .collect(),
        layout: input.layout,
    }
}

fn flavor<'t>(table: &'t FlavorTable, id: &str) -> &'t Flavor {
    table
        .flavor(id)
        .unwrap_or_else(|| panic!("flavor `{id}` is validated by the manifest"))
}

fn targeted_clients<'t>(table: &'t FlavorTable, input: &'t TocInput) -> Vec<&'t Client> {
    input
        .flavors
        .iter()
        .flat_map(|f| table.clients_of(&f.id))
        .collect()
}

/// The load-directive names for a flavor subset (§14.9.1, decision T44): a
/// family name when the subset contains every targeted flavor of that
/// family, otherwise the game-type names.
fn directive_names(
    table: &FlavorTable,
    subset: &[String],
    targets: &[FlavorTarget],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut families: Vec<Family> = Vec::new();
    for id in subset {
        let family = flavor(table, id).family;
        if !families.contains(&family) {
            families.push(family);
        }
    }
    for family in families {
        let targeted: Vec<&str> = targets
            .iter()
            .map(|t| t.id.as_str())
            .filter(|id| flavor(table, id).family == family)
            .collect();
        let chosen: Vec<&String> = subset
            .iter()
            .filter(|id| flavor(table, id).family == family)
            .collect();
        if targeted.iter().all(|t| chosen.iter().any(|c| c == t)) {
            names.push(family.name().to_string());
        } else {
            names.extend(chosen.into_iter().cloned());
        }
    }
    names
}

/// Why the single layout cannot be used, as `(client, feature)`.
fn single_layout_problem(table: &FlavorTable, input: &TocInput) -> Option<(String, String)> {
    let interface_count: usize = input.flavors.iter().map(|f| f.interfaces.len()).sum();
    let needs_file_directives = input
        .files
        .iter()
        .any(|f| f.flavors.is_some() || f.locale.is_some());
    let needs_directive_lines = input.directives.iter().any(|d| d.flavors.is_some());
    let needs_text_locale = input.files.iter().any(|f| f.locale.is_some());
    let mut names: Vec<String> = Vec::new();
    for subset in input
        .files
        .iter()
        .filter_map(|f| f.flavors.as_ref())
        .chain(input.directives.iter().filter_map(|d| d.flavors.as_ref()))
    {
        for name in directive_names(table, subset, &input.flavors) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }

    for client in targeted_clients(table, input) {
        let missing = if interface_count > 1 && !client.features.comma_interfaces {
            Some("comma-separated `## Interface:` values".to_string())
        } else if needs_file_directives && !client.features.inline_file_directives {
            Some("inline load directives on file lines".to_string())
        } else if needs_directive_lines && !client.features.inline_directive_lines {
            Some("inline load directives on `##` lines".to_string())
        } else if needs_text_locale && !client.features.allow_load_text_locale {
            Some("`[AllowLoadTextLocale]`".to_string())
        } else {
            names
                .iter()
                .find(|name| !client.recognizes_game_type(name))
                .map(|name| format!("the game type `{name}`"))
        };
        if let Some(feature) = missing {
            return Some((format!("{} {}", client.name, client.version), feature));
        }
    }
    None
}

fn directive_line(key: &str, value: &str, condition: Option<String>) -> String {
    match condition {
        Some(condition) => format!("## {key}: {value} {condition}"),
        None => format!("## {key}: {value}"),
    }
}

fn game_type_condition(names: &[String]) -> String {
    format!("[AllowLoadGameType {}]", names.join(" "))
}

fn render_single(table: &FlavorTable, input: &TocInput) -> TocFile {
    let interfaces: Vec<String> = input
        .flavors
        .iter()
        .flat_map(|f| f.interfaces.iter().map(u32::to_string))
        .collect();
    let mut lines = vec![format!("## Interface: {}", interfaces.join(", "))];
    for d in &input.directives {
        let condition = d
            .flavors
            .as_ref()
            .map(|subset| game_type_condition(&directive_names(table, subset, &input.flavors)));
        lines.extend(split_directive(&d.key, &d.value, condition));
    }
    for file in &input.files {
        let mut line = file.path.replace('/', "\\");
        if let Some(subset) = &file.flavors {
            line.push(' ');
            line.push_str(&game_type_condition(&directive_names(
                table,
                subset,
                &input.flavors,
            )));
        }
        if let Some(locale) = &file.locale {
            line.push_str(&format!(" [AllowLoadTextLocale {locale}]"));
        }
        lines.push(line);
    }
    TocFile {
        name: format!("{}.toc", input.addon_name),
        flavors: input.flavors.iter().map(|f| f.id.clone()).collect(),
        content: join_lines(&lines),
    }
}

fn applies(flavors: &Option<Vec<String>>, id: &str) -> bool {
    flavors
        .as_ref()
        .is_none_or(|list| list.iter().any(|f| f == id))
}

fn render_per_flavor(table: &FlavorTable, input: &TocInput) -> Result<Vec<TocFile>, Vec<TocError>> {
    let names = per_flavor_names(table, input)?;
    let mut out = Vec::new();
    for (target, name) in input.flavors.iter().zip(names) {
        let text_locale = table
            .clients_of(&target.id)
            .all(|c| c.features.allow_load_text_locale);
        let interfaces: Vec<String> = target.interfaces.iter().map(u32::to_string).collect();
        let mut lines = vec![format!("## Interface: {}", interfaces.join(", "))];
        for d in input
            .directives
            .iter()
            .filter(|d| applies(&d.flavors, &target.id))
        {
            lines.extend(split_directive(&d.key, &d.value, None));
        }
        for file in input
            .files
            .iter()
            .filter(|f| applies(&f.flavors, &target.id))
        {
            let mut line = file.path.replace('/', "\\");
            if let (Some(locale), true) = (&file.locale, text_locale) {
                line.push_str(&format!(" [AllowLoadTextLocale {locale}]"));
            }
            lines.push(line);
        }
        out.push(TocFile {
            name,
            flavors: vec![target.id.clone()],
            content: join_lines(&lines),
        });
    }
    Ok(out)
}

/// TOC file names for the per-flavor layout (§14.9.3).
fn per_flavor_names(table: &FlavorTable, input: &TocInput) -> Result<Vec<String>, Vec<TocError>> {
    if input.flavors.len() == 1 {
        return Ok(vec![format!("{}.toc", input.addon_name)]);
    }
    let mut family_suffix_taken: Vec<Family> = Vec::new();
    let mut names = Vec::new();
    for target in &input.flavors {
        let f = flavor(table, &target.id);
        let same_family = input
            .flavors
            .iter()
            .filter(|t| flavor(table, &t.id).family == f.family)
            .count();
        let clients: Vec<&Client> = table.clients_of(&f.id).collect();
        let game_suffix = f
            .game_suffixes
            .iter()
            .find(|s| clients.iter().all(|c| c.recognizes_suffix(s)));
        let suffix = if same_family == 1 {
            f.family.suffix().to_string()
        } else if let Some(suffix) = game_suffix {
            suffix.clone()
        } else if !family_suffix_taken.contains(&f.family) {
            family_suffix_taken.push(f.family);
            f.family.suffix().to_string()
        } else {
            let flavors = input
                .flavors
                .iter()
                .filter(|t| flavor(table, &t.id).family == f.family)
                .map(|t| t.id.clone())
                .collect();
            return Err(vec![TocError::Indistinguishable { flavors }]);
        };
        names.push(format!("{}{suffix}.toc", input.addon_name));
    }
    Ok(names)
}

/// Splits an appendable directive whose line would exceed the limit.
fn split_directive(key: &str, value: &str, condition: Option<String>) -> Vec<String> {
    let line = directive_line(key, value, condition.clone());
    if line.len() <= MAX_LINE_LENGTH || !APPENDABLE_DIRECTIVES.contains(&key) {
        return vec![line];
    }
    let mut lines = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let candidate = {
            let mut joined = current.clone();
            joined.push(item);
            directive_line(key, &joined.join(", "), condition.clone())
        };
        if candidate.len() > MAX_LINE_LENGTH && !current.is_empty() {
            lines.push(directive_line(key, &current.join(", "), condition.clone()));
            current.clear();
        }
        current.push(item);
    }
    if !current.is_empty() {
        lines.push(directive_line(key, &current.join(", "), condition));
    }
    lines
}

fn check_lines(toc: &TocFile) -> Vec<TocError> {
    toc.content
        .lines()
        .filter(|line| line.len() > MAX_LINE_LENGTH)
        .map(|line| TocError::LineTooLong {
            toc: toc.name.clone(),
            line: line.chars().take(60).collect(),
        })
        .collect()
}

fn join_lines(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str) -> FlavorTarget {
        let table = wowlua_flavors::table();
        FlavorTarget {
            id: id.into(),
            interfaces: table.flavor(id).unwrap().interface.clone(),
        }
    }

    fn input(flavors: &[&str]) -> TocInput {
        TocInput {
            addon_name: "MyAddon".into(),
            flavors: flavors.iter().map(|f| target(f)).collect(),
            directives: vec![
                Directive::new("Title", "My Addon"),
                Directive::new("Version", "1.0.0"),
            ],
            files: vec![
                FileEntry::new("src/core.lua"),
                FileEntry::new("src/ui/frames.xml"),
            ],
            layout: Layout::Auto,
        }
    }

    #[test]
    fn one_flavor_gives_a_plain_toc() {
        let generated = generate(&input(&["standard"])).unwrap();
        assert_eq!(generated.files.len(), 1);
        assert_eq!(generated.files[0].name, "MyAddon.toc");
        assert_eq!(
            generated.files[0].content,
            "## Interface: 120100\n## Title: My Addon\n## Version: 1.0.0\nsrc\\core.lua\nsrc\\ui\\frames.xml\n"
        );
    }

    #[test]
    fn shared_files_use_a_single_toc_with_all_interfaces() {
        let generated = generate(&input(&["standard", "mists", "vanilla"])).unwrap();
        assert_eq!(generated.layout, Layout::Single);
        assert!(
            generated.files[0]
                .content
                .starts_with("## Interface: 120100, 50504, 11509\n")
        );
    }

    #[test]
    fn flavor_files_use_family_names_when_unique() {
        let mut i = input(&["standard", "vanilla"]);
        i.files.push(FileEntry {
            path: "src/retail.lua".into(),
            flavors: Some(vec!["standard".into()]),
            locale: None,
        });
        let generated = generate(&i).unwrap();
        assert_eq!(generated.layout, Layout::Single);
        assert!(
            generated.files[0]
                .content
                .contains("src\\retail.lua [AllowLoadGameType mainline]\n")
        );
    }

    #[test]
    fn game_types_and_family_coverage() {
        let mut i = input(&["standard", "mists", "vanilla"]);
        i.files.push(FileEntry {
            path: "src/classic.lua".into(),
            flavors: Some(vec!["mists".into(), "vanilla".into()]),
            locale: None,
        });
        i.files.push(FileEntry {
            path: "src/era.lua".into(),
            flavors: Some(vec!["vanilla".into()]),
            locale: None,
        });
        let content = &generate(&i).unwrap().files[0].content;
        assert!(content.contains("src\\classic.lua [AllowLoadGameType classic]\n"));
        assert!(content.contains("src\\era.lua [AllowLoadGameType vanilla]\n"));
    }

    #[test]
    fn standard_cannot_be_named_for_classic_clients() {
        // `standard` and `camelot` share the Mainline family, so a
        // standard-only file needs the game type `standard`, which Classic
        // clients do not recognize: auto falls back to per-flavor TOCs.
        let mut i = input(&["standard", "camelot", "vanilla"]);
        i.files.push(FileEntry {
            path: "src/retail.lua".into(),
            flavors: Some(vec!["standard".into()]),
            locale: None,
        });
        let generated = generate(&i).unwrap();
        assert_eq!(generated.layout, Layout::PerFlavor);
        let names: Vec<&str> = generated.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "MyAddon_Mainline.toc",
                "MyAddon_Camelot.toc",
                "MyAddon_Classic.toc"
            ]
        );
        assert!(generated.files[0].content.contains("src\\retail.lua\n"));
        assert!(!generated.files[2].content.contains("retail.lua"));

        i.layout = Layout::Single;
        assert!(matches!(
            generate(&i).unwrap_err()[0],
            TocError::SingleUnsupported { .. }
        ));
    }

    #[test]
    fn per_flavor_suffixes_for_one_flavor_per_family() {
        let mut i = input(&["standard", "mists"]);
        i.layout = Layout::PerFlavor;
        let names: Vec<String> = generate(&i)
            .unwrap()
            .files
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["MyAddon_Mainline.toc", "MyAddon_Classic.toc"]);
    }

    #[test]
    fn classic_flavors_use_game_suffixes() {
        let mut i = input(&["mists", "vanilla", "tbc"]);
        i.layout = Layout::PerFlavor;
        let names: Vec<String> = generate(&i)
            .unwrap()
            .files
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(
            names,
            [
                "MyAddon_Mists.toc",
                "MyAddon_Vanilla.toc",
                "MyAddon_TBC.toc"
            ]
        );
    }

    #[test]
    fn cata_needs_per_flavor_tocs_for_flavor_files() {
        let mut i = input(&["standard", "cata"]);
        i.files.push(FileEntry {
            path: "src/cata.lua".into(),
            flavors: Some(vec!["cata".into()]),
            locale: None,
        });
        assert_eq!(generate(&i).unwrap().layout, Layout::PerFlavor);
    }

    #[test]
    fn appendable_directives_are_split_and_others_error() {
        let many: Vec<String> = (0..200).map(|i| format!("SavedVariable{i}")).collect();
        let mut i = input(&["standard"]);
        i.directives
            .push(Directive::new("SavedVariables", many.join(", ")));
        let content = generate(&i).unwrap().files.remove(0).content;
        assert!(content.matches("## SavedVariables:").count() > 1);
        assert!(content.lines().all(|l| l.len() <= MAX_LINE_LENGTH));

        let mut i = input(&["standard"]);
        i.directives.push(Directive::new("Notes", "x".repeat(2000)));
        assert!(matches!(
            generate(&i).unwrap_err()[0],
            TocError::LineTooLong { .. }
        ));
    }

    #[test]
    fn flavor_specific_directives() {
        let mut i = input(&["standard", "vanilla"]);
        i.directives.push(Directive {
            key: "OptionalDeps".into(),
            value: "Details".into(),
            flavors: Some(vec!["standard".into()]),
        });
        // The Retail beta client (12.0.1) predates inline directives on `##`
        // lines (12.0.7+), so auto falls back to per-flavor TOCs and the
        // directive only appears in the Retail TOC.
        let generated = generate(&i).unwrap();
        assert_eq!(generated.layout, Layout::PerFlavor);
        assert!(
            generated.files[0]
                .content
                .contains("## OptionalDeps: Details\n")
        );
        assert!(!generated.files[1].content.contains("OptionalDeps"));
    }
}
