//! Rendering diagnostics in the four message formats of the CLI
//! (`--message-format human|short|json|github`).

use crate::{Diagnostic, Severity};
use codespan_reporting::diagnostic as cs;
use codespan_reporting::files::{Error as FilesError, Files};
use codespan_reporting::term::{self, termcolor};
use std::io::{self, Write};
use std::ops::Range;
use wowlua_span::{FileId, SourceDb};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageFormat {
    #[default]
    Human,
    Short,
    Json,
    Github,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

/// Renders diagnostics in one message format.
#[derive(Clone, Copy, Debug, Default)]
pub struct Renderer {
    pub format: MessageFormat,
    pub color: ColorChoice,
}

impl Renderer {
    pub fn new(format: MessageFormat, color: ColorChoice) -> Self {
        Renderer { format, color }
    }

    /// Writes all diagnostics to stderr.
    pub fn emit_stderr(&self, db: &SourceDb, diagnostics: &[Diagnostic]) -> io::Result<()> {
        let choice = match self.color {
            ColorChoice::Always => termcolor::ColorChoice::Always,
            ColorChoice::Never => termcolor::ColorChoice::Never,
            ColorChoice::Auto if std::env::var_os("NO_COLOR").is_some() => {
                termcolor::ColorChoice::Never
            }
            ColorChoice::Auto => termcolor::ColorChoice::Auto,
        };
        let stream = termcolor::StandardStream::stderr(choice);
        let mut out = stream.lock();
        for diagnostic in diagnostics {
            self.write(db, diagnostic, &mut out)?;
        }
        out.flush()
    }

    /// Renders one diagnostic without colors (used by tests and logs).
    pub fn render_to_string(&self, db: &SourceDb, diagnostic: &Diagnostic) -> String {
        let mut buffer = termcolor::Buffer::no_color();
        self.write(db, diagnostic, &mut buffer)
            .expect("writing to a memory buffer cannot fail");
        String::from_utf8_lossy(buffer.as_slice()).into_owned()
    }

    fn write(
        &self,
        db: &SourceDb,
        diagnostic: &Diagnostic,
        out: &mut dyn termcolor::WriteColor,
    ) -> io::Result<()> {
        match self.format {
            MessageFormat::Human => write_human(db, diagnostic, out),
            MessageFormat::Short => writeln!(out, "{}", short_line(db, diagnostic)),
            MessageFormat::Json => writeln!(out, "{}", json_line(db, diagnostic)),
            MessageFormat::Github => writeln!(out, "{}", github_line(db, diagnostic)),
        }
    }
}

/// Where a diagnostic points: display path plus one-based line and column.
struct Location {
    path: String,
    start: Option<(u32, u32)>,
    end: Option<(u32, u32)>,
}

fn location(db: &SourceDb, diagnostic: &Diagnostic) -> Option<Location> {
    if let Some(label) = diagnostic.labels.first() {
        let file = db.get(label.span.file);
        let start = file.line_col(label.span.start as usize);
        let end = file.line_col(label.span.end as usize);
        return Some(Location {
            path: display_path(file.path()),
            start: Some((start.line, start.column)),
            end: Some((end.line, end.column)),
        });
    }
    diagnostic.path.as_ref().map(|path| Location {
        path: display_path(path),
        start: None,
        end: None,
    })
}

fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn header(diagnostic: &Diagnostic) -> String {
    match diagnostic.code {
        Some(code) => format!("{}[{code}]", diagnostic.severity.as_str()),
        None => diagnostic.severity.as_str().to_string(),
    }
}

fn short_line(db: &SourceDb, diagnostic: &Diagnostic) -> String {
    let prefix = match location(db, diagnostic) {
        Some(Location {
            path,
            start: Some((line, column)),
            ..
        }) => format!("{path}:{line}:{column}: "),
        Some(Location { path, .. }) => format!("{path}: "),
        None => String::new(),
    };
    format!("{prefix}{}: {}", header(diagnostic), diagnostic.message)
}

fn json_line(db: &SourceDb, diagnostic: &Diagnostic) -> String {
    let mut object = serde_json::Map::new();
    object.insert("severity".into(), diagnostic.severity.as_str().into());
    object.insert(
        "code".into(),
        diagnostic.code.map_or(serde_json::Value::Null, Into::into),
    );
    object.insert("message".into(), diagnostic.message.clone().into());
    if let Some(loc) = location(db, diagnostic) {
        object.insert("file".into(), loc.path.into());
        if let (Some(start), Some(end)) = (loc.start, loc.end) {
            object.insert("line".into(), start.0.into());
            object.insert("column".into(), start.1.into());
            object.insert("end_line".into(), end.0.into());
            object.insert("end_column".into(), end.1.into());
        }
    }
    let labels: Vec<serde_json::Value> = diagnostic
        .labels
        .iter()
        .skip(1)
        .map(|label| {
            let file = db.get(label.span.file);
            let start = file.line_col(label.span.start as usize);
            serde_json::json!({
                "file": display_path(file.path()),
                "line": start.line,
                "column": start.column,
                "message": label.message,
            })
        })
        .collect();
    if !labels.is_empty() {
        object.insert("labels".into(), labels.into());
    }
    object.insert("notes".into(), diagnostic.notes.clone().into());
    if let Some(help) = &diagnostic.help {
        object.insert("help".into(), help.clone().into());
    }
    serde_json::Value::Object(object).to_string()
}

fn github_line(db: &SourceDb, diagnostic: &Diagnostic) -> String {
    let command = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "notice",
    };
    let mut properties = Vec::new();
    if let Some(loc) = location(db, diagnostic) {
        properties.push(format!("file={}", escape_property(&loc.path)));
        if let Some((line, column)) = loc.start {
            properties.push(format!("line={line}"));
            properties.push(format!("col={column}"));
        }
        if let Some((line, column)) = loc.end {
            properties.push(format!("endLine={line}"));
            properties.push(format!("endColumn={column}"));
        }
    }
    if let Some(code) = diagnostic.code {
        properties.push(format!("title={code}"));
    }
    let mut message = diagnostic.message.clone();
    for note in &diagnostic.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    if let Some(help) = &diagnostic.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    let properties = if properties.is_empty() {
        String::new()
    } else {
        format!(" {}", properties.join(","))
    };
    format!("::{command}{properties}::{}", escape_data(&message))
}

fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

fn write_human(
    db: &SourceDb,
    diagnostic: &Diagnostic,
    out: &mut dyn termcolor::WriteColor,
) -> io::Result<()> {
    let severity = match diagnostic.severity {
        Severity::Error => cs::Severity::Error,
        Severity::Warning => cs::Severity::Warning,
        Severity::Note => cs::Severity::Note,
    };
    let labels = diagnostic
        .labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == 0 {
                cs::LabelStyle::Primary
            } else {
                cs::LabelStyle::Secondary
            };
            cs::Label::new(style, label.span.file, label.span.range()).with_message(&label.message)
        })
        .collect();
    let mut notes = Vec::new();
    if diagnostic.labels.is_empty()
        && let Some(path) = &diagnostic.path
    {
        notes.push(format!("in {}", display_path(path)));
    }
    notes.extend(diagnostic.notes.iter().map(|n| format!("note: {n}")));
    if let Some(help) = &diagnostic.help {
        notes.push(format!("help: {help}"));
    }
    let mut rendered = cs::Diagnostic::new(severity)
        .with_message(&diagnostic.message)
        .with_labels(labels)
        .with_notes(notes);
    if let Some(code) = diagnostic.code {
        rendered = rendered.with_code(code);
    }
    let config = term::Config::default();
    term::emit_to_write_style(out, &config, &DbFiles(db), &rendered).map_err(|e| match e {
        FilesError::Io(io) => io,
        other => io::Error::other(other.to_string()),
    })
}

/// Adapter that lets codespan-reporting read files from a [`SourceDb`].
struct DbFiles<'a>(&'a SourceDb);

impl<'a> Files<'a> for DbFiles<'a> {
    type FileId = FileId;
    type Name = String;
    type Source = &'a str;

    fn name(&'a self, id: FileId) -> Result<String, FilesError> {
        Ok(display_path(self.0.get(id).path()))
    }

    fn source(&'a self, id: FileId) -> Result<&'a str, FilesError> {
        Ok(self.0.get(id).text())
    }

    fn line_index(&'a self, id: FileId, byte_index: usize) -> Result<usize, FilesError> {
        Ok(self.0.get(id).line_index(byte_index))
    }

    fn line_range(&'a self, id: FileId, line_index: usize) -> Result<Range<usize>, FilesError> {
        let file = self.0.get(id);
        let starts = file.line_starts();
        let start = *starts.get(line_index).ok_or(FilesError::LineTooLarge {
            given: line_index,
            max: starts.len() - 1,
        })?;
        let end = starts
            .get(line_index + 1)
            .copied()
            .unwrap_or(file.text().len());
        Ok(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wowlua_span::Span;

    fn sample() -> (SourceDb, Diagnostic) {
        let mut db = SourceDb::new();
        let file = db.add("wowlua.json", "{\n  \"addon\": { \"titel\": \"x\" }\n}\n");
        let diagnostic = Diagnostic::error("E4002", "unknown key `titel` in `addon`")
            .with_label(Span::new(file, 16..23), "unknown key")
            .with_help("did you mean `title`?");
        (db, diagnostic)
    }

    #[test]
    fn short_format() {
        let (db, d) = sample();
        let out = Renderer::new(MessageFormat::Short, ColorChoice::Never).render_to_string(&db, &d);
        assert_eq!(
            out,
            "wowlua.json:2:15: error[E4002]: unknown key `titel` in `addon`\n"
        );
    }

    #[test]
    fn github_format_escapes() {
        let (db, d) = sample();
        let out =
            Renderer::new(MessageFormat::Github, ColorChoice::Never).render_to_string(&db, &d);
        assert!(out.starts_with(
            "::error file=wowlua.json,line=2,col=15,endLine=2,endColumn=22,title=E4002::"
        ));
        assert!(out.contains("%0Ahelp: did you mean"));
    }

    #[test]
    fn json_format_is_one_object_per_line() {
        let (db, d) = sample();
        let out = Renderer::new(MessageFormat::Json, ColorChoice::Never).render_to_string(&db, &d);
        let value: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(value["code"], "E4002");
        assert_eq!(value["line"], 2);
    }

    #[test]
    fn human_format_shows_source() {
        let (db, d) = sample();
        let out = Renderer::new(MessageFormat::Human, ColorChoice::Never).render_to_string(&db, &d);
        assert!(out.contains("error[E4002]: unknown key"));
        assert!(out.contains("wowlua.json:2:15"));
        assert!(out.contains("help: did you mean"));
    }

    #[test]
    fn path_only_diagnostics() {
        let db = SourceDb::new();
        let d = Diagnostic::plain_error("file not found").with_path("src/main.lua");
        let out = Renderer::new(MessageFormat::Short, ColorChoice::Never).render_to_string(&db, &d);
        assert_eq!(out, "src/main.lua: error: file not found\n");
    }
}
