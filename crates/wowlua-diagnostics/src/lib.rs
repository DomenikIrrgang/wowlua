//! Diagnostics: the error/warning model shared by every wowlua crate, the
//! registry of diagnostic codes (spec Appendix B) with their explanations,
//! and rendering in the `human`, `short`, `json` and `github` message formats.

mod codes;
mod explanations;
mod render;

use std::path::{Path, PathBuf};
use wowlua_span::Span;

pub use explanations::{Example, ExampleFile, Explanation, areas, explanation};
pub use render::{ColorChoice, MessageFormat, Renderer};

/// A registered diagnostic code and its one-line summary.
#[derive(Debug, Clone, Copy)]
pub struct CodeInfo {
    pub code: &'static str,
    pub summary: &'static str,
}

/// Looks up a code such as `"E4002"` in the registry generated from the spec.
pub fn lookup(code: &str) -> Option<&'static CodeInfo> {
    let code = code.to_ascii_uppercase();
    codes::CODES.iter().find(|info| info.code == code)
}

/// All registered codes, sorted by number.
pub fn all_codes() -> &'static [CodeInfo] {
    codes::CODES
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Note,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// A source location attached to a diagnostic.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A text replacement in one file.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

/// A machine-applicable fix (used by `wowlua fix` and the LSP).
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub message: String,
    pub edits: Vec<Edit>,
}

/// The lint codes (`wowlua lint`, `--!nolint`).
pub const LINT_CODES: &[&str] = &["W1003", "W1004", "W1005"];

/// One error, warning or note.
///
/// The first label is the primary location. A diagnostic without labels may
/// still name a file through `path` (for example "file not found").
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<&'static str>,
    pub message: String,
    pub labels: Vec<Label>,
    pub path: Option<PathBuf>,
    pub notes: Vec<String>,
    pub help: Option<String>,
    pub fixes: Vec<Fix>,
}

impl Diagnostic {
    /// An error with a registered code. The code's letter must be `E`.
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        debug_assert!(code.starts_with('E'), "{code} is not an error code");
        Self::new(Severity::Error, Some(code), message)
    }

    /// A warning with a registered code. The code's letter must be `W`.
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        debug_assert!(code.starts_with('W'), "{code} is not a warning code");
        Self::new(Severity::Warning, Some(code), message)
    }

    /// An error without a code, for problems outside the language (I/O, usage).
    pub fn plain_error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, None, message)
    }

    pub fn plain_warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, None, message)
    }

    pub fn note(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, None, message)
    }

    fn new(severity: Severity, code: Option<&'static str>, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            code,
            message: message.into(),
            labels: Vec::new(),
            path: None,
            notes: Vec::new(),
            help: None,
            fixes: Vec::new(),
        }
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_fix(mut self, message: impl Into<String>, edits: Vec<Edit>) -> Self {
        self.fixes.push(Fix {
            message: message.into(),
            edits,
        });
        self
    }

    /// A lint diagnostic (W1003, W1004, W1005).
    pub fn is_lint(&self) -> bool {
        self.code.is_some_and(|c| LINT_CODES.contains(&c))
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// How a lint code is treated, from `check.lints` in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintLevel {
    Allow,
    Warn,
    Deny,
}

/// An ordered collection of diagnostics produced by one command.
#[derive(Debug, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.items.extend(other);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// True when the command must fail: any error, or any warning with
    /// `--deny-warnings`.
    pub fn should_fail(&self, deny_warnings: bool) -> bool {
        self.has_errors() || (deny_warnings && self.warning_count() > 0)
    }

    /// Applies `check.lints` overrides: `allow` drops a warning, `deny` turns it
    /// into an error, `warn` keeps it. Errors are never downgraded.
    pub fn apply_lint_levels(&mut self, level_of: impl Fn(&str) -> Option<LintLevel>) {
        self.items.retain_mut(|d| {
            let Some(code) = d.code else { return true };
            if d.severity != Severity::Warning {
                return true;
            }
            match level_of(code) {
                Some(LintLevel::Allow) => false,
                Some(LintLevel::Deny) => {
                    d.severity = Severity::Error;
                    true
                }
                Some(LintLevel::Warn) | None => true,
            }
        });
    }

    pub fn retain(&mut self, keep: impl FnMut(&Diagnostic) -> bool) {
        self.items.retain(keep);
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_spec_codes() {
        assert!(lookup("E4002").is_some());
        assert!(lookup("w1001").is_some());
        assert!(lookup("E9999").is_none());
        let codes = all_codes();
        assert!(codes.windows(2).all(|w| w[0].code[1..] <= w[1].code[1..]));
    }

    #[test]
    fn lint_levels_allow_and_deny_warnings() {
        let mut diags = Diagnostics::new();
        diags.push(Diagnostic::warning("W1003", "unused"));
        diags.push(Diagnostic::warning("W1004", "shadowed"));
        diags.push(Diagnostic::error("E2001", "mismatch"));
        diags.apply_lint_levels(|code| match code {
            "W1003" => Some(LintLevel::Allow),
            "W1004" => Some(LintLevel::Deny),
            _ => None,
        });
        assert_eq!(diags.error_count(), 2);
        assert_eq!(diags.warning_count(), 0);
    }
}
