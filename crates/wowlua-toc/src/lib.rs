//! TOC files: generation (spec §14.9) and parsing (for `wowlua init`).

mod directives;
mod generate;
mod parse;

pub use directives::{
    EXTRA_DIRECTIVES, GENERATED_DIRECTIVES, KnownDirective, extra_directives, generated_directives,
    is_generated_directive, is_known_extra_directive,
};

pub use generate::{
    Directive, FileEntry, FlavorTarget, Layout, TocError, TocFile, TocInput, generate,
};
pub use parse::{
    InlineDirective, ParsedDirective, ParsedFile, ParsedToc, parse, split_conditions,
    split_file_name,
};

/// The client reads only the first 1024 characters of each TOC line.
pub const MAX_LINE_LENGTH: usize = 1024;

/// Directives the client merges when repeated, so long values may be split
/// across several lines (§14.9.4).
pub const APPENDABLE_DIRECTIVES: &[&str] = &[
    "Dependencies",
    "OptionalDeps",
    "LoadWith",
    "LoadManagers",
    "SavedVariables",
    "SavedVariablesPerCharacter",
];
