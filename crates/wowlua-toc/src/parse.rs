//! Parsing existing TOC files (used by `wowlua init` to adopt an addon).

/// An inline load directive such as `[AllowLoadGameType mainline classic]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineDirective {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedDirective {
    pub key: String,
    pub value: String,
    pub conditions: Vec<InlineDirective>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedFile {
    /// The path as written, with `\` normalized to `/`.
    pub path: String,
    pub conditions: Vec<InlineDirective>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedToc {
    pub directives: Vec<ParsedDirective>,
    pub files: Vec<ParsedFile>,
}

impl ParsedToc {
    /// The first value of a directive (case-insensitive key).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.directives
            .iter()
            .find(|d| d.key.eq_ignore_ascii_case(key))
            .map(|d| d.value.as_str())
    }

    /// All values of a repeatable directive, split on commas, for every key
    /// accepted by `matches`.
    pub fn list(&self, matches: impl Fn(&str) -> bool) -> Vec<String> {
        self.directives
            .iter()
            .filter(|d| matches(&d.key))
            .flat_map(|d| d.value.split(','))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Interface numbers from `## Interface:` (comma-separated).
    pub fn interfaces(&self) -> Vec<u32> {
        self.list(|k| k.eq_ignore_ascii_case("Interface"))
            .iter()
            .filter_map(|v| v.parse().ok())
            .collect()
    }
}

const INLINE_DIRECTIVES: &[&str] = &[
    "AllowLoadGameType",
    "ExcludeLoadGameType",
    "AllowLoadTextLocale",
    "AllowLoad",
    "AllowLoadEnvironment",
    "LoadIntoEnvironment",
    "Bootstrap",
];

/// Parses TOC text. Comments (including packager `#@…@` lines) and blank
/// lines are skipped.
pub fn parse(text: &str) -> ParsedToc {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut toc = ParsedToc::default();
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("##") {
            let Some((key, value)) = rest.split_once(':') else {
                continue;
            };
            let (value, conditions) = split_conditions(value.trim());
            toc.directives.push(ParsedDirective {
                key: key.trim().to_string(),
                value,
                conditions,
            });
        } else if line.starts_with('#') {
            continue;
        } else {
            let (path, conditions) = split_conditions(line);
            toc.files.push(ParsedFile {
                path: path.replace('\\', "/"),
                conditions,
            });
        }
    }
    toc
}

/// Splits inline directives (`[Name values]`, also written `[Name: values]`)
/// off a TOC line or an XML `file` attribute. They may stand anywhere in the
/// line. Brackets that are not directives (`Locales\[TextLocale].lua`) are
/// kept.
pub fn split_conditions(line: &str) -> (String, Vec<InlineDirective>) {
    let mut kept = String::new();
    let mut conditions = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(len) = rest[open..].find(']') else {
            break;
        };
        let close = open + len;
        let mut words = rest[open + 1..close]
            .split([' ', '\t', ',', ':'])
            .filter(|w| !w.is_empty());
        match words.next() {
            Some(name)
                if INLINE_DIRECTIVES
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(name)) =>
            {
                conditions.push(InlineDirective {
                    name: name.to_string(),
                    values: words.map(str::to_string).collect(),
                });
                kept.push_str(rest[..open].trim_end());
            }
            _ => kept.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    kept.push_str(rest);
    (kept.trim().to_string(), conditions)
}

/// Splits a TOC file name into the addon name and its flavor suffix:
/// `MyAddon_Classic.toc` → (`MyAddon`, `Some("_Classic")`). Suffixes are
/// recognized from the flavor table, including legacy `-BCC` forms.
pub fn split_file_name(file_name: &str) -> Option<(String, Option<String>)> {
    let stem = file_name
        .strip_suffix(".toc")
        .or_else(|| file_name.strip_suffix(".TOC"))?;
    let table = wowlua_flavors::table();
    for (i, c) in stem.char_indices().rev() {
        if c == '_' || c == '-' {
            let suffix = &stem[i..];
            if table.suffix_meaning(suffix).is_some() {
                return Some((stem[..i].to_string(), Some(suffix.to_string())));
            }
        }
    }
    Some((stem.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_form_and_any_position() {
        let (path, conditions) = split_conditions("Data_TBC.lua [AllowLoadGameType: tbc]");
        assert_eq!(path, "Data_TBC.lua");
        assert_eq!(conditions[0].name, "AllowLoadGameType");
        assert_eq!(conditions[0].values, ["tbc"]);
        let (value, conditions) = split_conditions("A [AllowLoadGameType:vanilla] B");
        assert_eq!(value, "A B");
        assert_eq!(conditions[0].values, ["vanilla"]);
        let (path, conditions) = split_conditions("Locales\\[TextLocale].lua");
        assert_eq!(path, "Locales\\[TextLocale].lua");
        assert!(conditions.is_empty());
    }

    #[test]
    fn parses_directives_files_and_conditions() {
        let text = "\u{feff}## Interface: 120100, 11509\r\n## Title: My Addon\n## Dep: Blizzard_X [AllowLoadGameType mainline]\n# comment\n#@debug@\nembeds.xml\nLocales\\[TextLocale].lua\nretail\\Edit.lua [AllowLoadGameType standard, camelot]\n\nlocales\\deDE.lua [AllowLoadTextLocale deDE] [AllowLoadGameType classic]\n";
        let toc = parse(text);
        assert_eq!(toc.interfaces(), vec![120100, 11509]);
        assert_eq!(toc.get("title"), Some("My Addon"));
        assert_eq!(toc.directives[2].value, "Blizzard_X");
        assert_eq!(toc.directives[2].conditions[0].values, vec!["mainline"]);
        let paths: Vec<&str> = toc.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "embeds.xml",
                "Locales/[TextLocale].lua",
                "retail/Edit.lua",
                "locales/deDE.lua"
            ]
        );
        assert_eq!(
            toc.files[2].conditions[0].values,
            vec!["standard", "camelot"]
        );
        assert_eq!(toc.files[3].conditions.len(), 2);
        assert_eq!(toc.files[3].conditions[0].name, "AllowLoadTextLocale");
    }

    #[test]
    fn leading_inline_directives() {
        let toc = parse("## OptionalDeps: [ExcludeLoadGameType vanilla] Ace3, Masque\n");
        assert_eq!(toc.directives[0].value, "Ace3, Masque");
        assert_eq!(toc.directives[0].conditions[0].name, "ExcludeLoadGameType");
        assert_eq!(toc.directives[0].conditions[0].values, vec!["vanilla"]);
    }

    #[test]
    fn adjacent_inline_directives() {
        let toc = parse(
            "Game\\load_forever.xml [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]\n",
        );
        assert_eq!(toc.files[0].path, "Game/load_forever.xml");
        assert_eq!(toc.files[0].conditions.len(), 2);
        assert_eq!(
            toc.files[0].conditions[1].values,
            vec!["standard", "classic"]
        );
    }

    #[test]
    fn splits_file_names() {
        assert_eq!(
            split_file_name("MyAddon.toc"),
            Some(("MyAddon".into(), None))
        );
        assert_eq!(
            split_file_name("My_Addon_Classic.toc"),
            Some(("My_Addon".into(), Some("_Classic".into())))
        );
        assert_eq!(
            split_file_name("MyAddon-BCC.toc"),
            Some(("MyAddon".into(), Some("-BCC".into())))
        );
        assert_eq!(
            split_file_name("MyAddon_TBC.toc"),
            Some(("MyAddon".into(), Some("_TBC".into())))
        );
        assert_eq!(
            split_file_name("MyAddon_Mists.toc"),
            Some(("MyAddon".into(), Some("_Mists".into())))
        );
        // Suffixes of games without a flavor are part of the name.
        assert_eq!(
            split_file_name("MyAddon_Cata.toc"),
            Some(("MyAddon_Cata".into(), None))
        );
        assert_eq!(split_file_name("readme.md"), None);
    }

    #[test]
    fn lists_repeatable_directives() {
        let toc = parse("## Dependencies: A, B\n## RequiredDeps: C\n## OptionalDeps: D\n");
        let required = toc.list(|k| k.starts_with("Dep") || k == "RequiredDeps");
        assert_eq!(required, vec!["A", "B", "C"]);
    }
}
