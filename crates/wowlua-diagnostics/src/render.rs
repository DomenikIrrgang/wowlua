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
        self.emit_stderr_with_summary(db, diagnostics, false)
    }

    /// [`Renderer::emit_stderr`], then with `summary`, in the human and
    /// short formats, a line counting the errors and warnings (as rustc
    /// ends its output).
    pub fn emit_stderr_with_summary(
        &self,
        db: &SourceDb,
        diagnostics: &[Diagnostic],
        summary: bool,
    ) -> io::Result<()> {
        let choice = match self.color {
            ColorChoice::Always => termcolor::ColorChoice::Always,
            ColorChoice::Never => termcolor::ColorChoice::Never,
            ColorChoice::Auto if std::env::var_os("NO_COLOR").is_some() => {
                termcolor::ColorChoice::Never
            }
            ColorChoice::Auto => termcolor::ColorChoice::Auto,
        };
        // Buffered: rendering writes many small pieces, and stderr is
        // unbuffered, so each was its own write call; with tens of MB of
        // diagnostics those calls took much of the run. Same bytes.
        let mut out = termcolor::BufferedStandardStream::stderr(choice);
        for diagnostic in diagnostics {
            self.write(db, diagnostic, &mut out)?;
        }
        if summary
            && matches!(self.format, MessageFormat::Human | MessageFormat::Short)
            && let Some(line) = summary_line(diagnostics)
        {
            self.write(db, &line, &mut out)?;
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

/// The closing line of a run with errors or warnings, worded as rustc's:
/// `warning: 2 warnings emitted`, or `error: aborting due to 3 previous
/// errors; 2 warnings emitted`.
pub fn summary_line(diagnostics: &[Diagnostic]) -> Option<Diagnostic> {
    let count = |severity| {
        diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    };
    let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let warnings_text = plural(warnings, "warning") + " emitted";
    match (errors, warnings) {
        (0, 0) => None,
        (0, _) => Some(Diagnostic::plain_warning(warnings_text)),
        (_, 0) => Some(Diagnostic::plain_error(format!(
            "aborting due to {}",
            plural(errors, "previous error")
        ))),
        _ => Some(Diagnostic::plain_error(format!(
            "aborting due to {}; {warnings_text}",
            plural(errors, "previous error")
        ))),
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
    let fixes: Vec<serde_json::Value> = diagnostic
        .fixes
        .iter()
        .map(|fix| {
            let edits: Vec<serde_json::Value> = fix
                .edits
                .iter()
                .map(|edit| {
                    let file = db.get(edit.span.file);
                    let start = file.line_col(edit.span.start as usize);
                    let end = file.line_col(edit.span.end as usize);
                    serde_json::json!({
                        "file": display_path(file.path()),
                        "line": start.line,
                        "column": start.column,
                        "end_line": end.line,
                        "end_column": end.column,
                        "replacement": edit.replacement,
                    })
                })
                .collect();
            serde_json::json!({ "message": fix.message, "edits": edits })
        })
        .collect();
    if !fixes.is_empty() {
        object.insert("fixes".into(), fixes.into());
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
    let result = match Windowed::new(db, diagnostic) {
        // A line too long to show: a window of it around the labels.
        Some(windowed) => {
            rendered.labels = windowed.labels(&rendered.labels);
            term::emit_to_write_style(out, &config, &windowed, &rendered)
        }
        None => term::emit_to_write_style(out, &config, &DbFiles(db), &rendered),
    };
    result.map_err(|e| match e {
        FilesError::Io(io) => io,
        other => io::Error::other(other.to_string()),
    })
}

/// Source lines longer than this many characters are shown as a window
/// around their labels, as rustc and Clang do: data files can have lines of
/// hundreds of KB, and each diagnostic would print the whole line.
const LONG_LINE: usize = 160;
/// How many characters of a long line are shown.
const WINDOW: usize = 120;
/// How many characters before the first label the window starts.
const BEFORE: usize = 40;
/// How many characters after the last label the window goes on, at least.
const AFTER: usize = 10;

/// The byte offset `n` characters before `at` in `line` (or 0).
fn chars_back(line: &str, at: usize, n: usize) -> usize {
    line[..at]
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map_or(0, |(i, _)| i)
}

/// The byte offset `n` characters after `at` in `line` (or its end).
fn chars_forward(line: &str, at: usize, n: usize) -> usize {
    line[at..]
        .char_indices()
        .nth(n)
        .map_or(line.len(), |(i, _)| at + i)
}

/// One line of a [`Windowed`] file.
struct WindowLine {
    /// The original line index.
    index: usize,
    /// Its byte offset in the original text.
    start: usize,
    /// The bytes of its content (without the line break) that are shown.
    shown: Range<usize>,
    /// Bytes of `...` before the shown part.
    prefix: usize,
    /// Where the line starts and ends in the window text.
    text: Range<usize>,
}

/// A diagnostic's files as codespan sees them when a line it shows is too
/// long: only the lines it shows (and the lines next to them), long ones cut
/// to a window around the labels. Line and column numbers are the original
/// ones.
struct Windowed<'a> {
    db: &'a SourceDb,
    files: Vec<(FileId, String, Vec<WindowLine>)>,
}

impl<'a> Windowed<'a> {
    /// `None` when every line the diagnostic shows is short enough.
    fn new(db: &'a SourceDb, diagnostic: &Diagnostic) -> Option<Windowed<'a>> {
        // Per file and line: the label boundaries on it.
        let mut lines: std::collections::BTreeMap<(FileId, usize), Vec<usize>> = Default::default();
        for label in &diagnostic.labels {
            let file = db.get(label.span.file);
            let (start, end) = (label.span.start as usize, label.span.end as usize);
            let (first, last) = (file.line_index(start), file.line_index(end));
            let max = file.line_starts().len() - 1;
            // Codespan also shows a line between two shown ones.
            for line in first.saturating_sub(1)..=(last + 1).min(max) {
                lines.entry((label.span.file, line)).or_default();
            }
            lines
                .entry((label.span.file, first))
                .or_default()
                .push(start);
            lines.entry((label.span.file, last)).or_default().push(end);
        }
        let content = |file: FileId, line: usize| {
            let f = db.get(file);
            let starts = f.line_starts();
            let start = starts[line];
            let end = starts.get(line + 1).copied().unwrap_or(f.text().len());
            let text = &f.text()[start..end];
            let body = text.trim_end_matches(['\n', '\r']);
            (start, body, &text[body.len()..])
        };
        let long = |body: &str| body.len() > LONG_LINE && body.chars().count() > LONG_LINE;
        if !lines
            .keys()
            .any(|&(file, line)| long(content(file, line).1))
        {
            return None;
        }
        let mut files: Vec<(FileId, String, Vec<WindowLine>)> = Vec::new();
        for ((file, index), points) in lines {
            if files.last().is_none_or(|(f, ..)| *f != file) {
                files.push((file, String::new(), Vec::new()));
            }
            let (_, text, rows) = files.last_mut().expect("pushed");
            let (start, body, ending) = content(file, index);
            let shown = if !long(body) {
                0..body.len()
            } else {
                let offsets: Vec<usize> =
                    points.iter().map(|p| (p - start).min(body.len())).collect();
                let lo = offsets.iter().copied().min().unwrap_or(0);
                let hi = offsets.iter().copied().max().unwrap_or(0);
                let from = if offsets.is_empty() {
                    0
                } else {
                    chars_back(body, lo, BEFORE)
                };
                let to = chars_forward(body, from, WINDOW).max(chars_forward(body, hi, AFTER));
                from..to
            };
            let begin = text.len();
            let prefix = if shown.start > 0 { "..." } else { "" };
            text.push_str(prefix);
            text.push_str(&body[shown.clone()]);
            if shown.end < body.len() {
                text.push_str("...");
            }
            text.push_str(ending);
            rows.push(WindowLine {
                index,
                start,
                shown,
                prefix: prefix.len(),
                text: begin..text.len(),
            });
        }
        Some(Windowed { db, files })
    }

    fn file(&self, id: FileId) -> Result<&(FileId, String, Vec<WindowLine>), FilesError> {
        self.files
            .iter()
            .find(|(f, ..)| *f == id)
            .ok_or(FilesError::FileMissing)
    }

    /// The window text's offset of original byte `offset` of `id`. Label
    /// boundaries are inside their line's window; a position in the line
    /// break stays in the line break.
    fn map(&self, id: FileId, offset: usize) -> usize {
        let line = self.db.get(id).line_index(offset);
        let Ok(row) = self.row(id, line) else {
            return 0;
        };
        let at = offset - row.start;
        let shown_end = row.text.start + row.prefix + row.shown.len();
        if at <= row.shown.start {
            row.text.start + row.prefix
        } else if at <= row.shown.end {
            row.text.start + row.prefix + (at - row.shown.start)
        } else if row.shown.end == self.body_len(id, row) {
            (shown_end + at - row.shown.end).min(row.text.end)
        } else {
            shown_end
        }
    }

    fn body_len(&self, id: FileId, row: &WindowLine) -> usize {
        let f = self.db.get(id);
        let starts = f.line_starts();
        let end = starts.get(row.index + 1).copied().unwrap_or(f.text().len());
        f.text()[row.start..end]
            .trim_end_matches(['\n', '\r'])
            .len()
    }

    /// The labels with their ranges in the window texts.
    fn labels(&self, labels: &[cs::Label<FileId>]) -> Vec<cs::Label<FileId>> {
        labels
            .iter()
            .map(|l| {
                let mut l = l.clone();
                l.range = self.map(l.file_id, l.range.start)..self.map(l.file_id, l.range.end);
                l
            })
            .collect()
    }

    fn row(&self, id: FileId, line_index: usize) -> Result<&WindowLine, FilesError> {
        let (_, _, rows) = self.file(id)?;
        rows.iter()
            .find(|r| r.index == line_index)
            .ok_or(FilesError::LineTooLarge {
                given: line_index,
                max: rows.last().map_or(0, |r| r.index),
            })
    }
}

impl<'a> Files<'a> for Windowed<'a> {
    type FileId = FileId;
    type Name = String;
    type Source = &'a str;

    fn name(&'a self, id: FileId) -> Result<String, FilesError> {
        Ok(display_path(self.db.get(id).path()))
    }

    fn source(&'a self, id: FileId) -> Result<&'a str, FilesError> {
        Ok(&self.file(id)?.1)
    }

    fn line_index(&'a self, id: FileId, byte_index: usize) -> Result<usize, FilesError> {
        let (_, _, rows) = self.file(id)?;
        let at = rows.partition_point(|r| r.text.start <= byte_index);
        Ok(rows[at.saturating_sub(1)].index)
    }

    fn line_range(&'a self, id: FileId, line_index: usize) -> Result<Range<usize>, FilesError> {
        Ok(self.row(id, line_index)?.text.clone())
    }

    /// The column in the original line, as codespan computes it there.
    fn column_number(
        &'a self,
        id: FileId,
        line_index: usize,
        byte_index: usize,
    ) -> Result<usize, FilesError> {
        let row = self.row(id, line_index)?;
        let in_text = byte_index.saturating_sub(row.text.start);
        let at = row.shown.start + in_text.saturating_sub(row.prefix).min(row.shown.len());
        let original = DbFiles(self.db);
        let range = original.line_range(id, line_index)?;
        Ok(
            codespan_reporting::files::column_index(self.db.get(id).text(), range, row.start + at)
                + 1,
        )
    }
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

    /// The human format without windowing, as it rendered every line before.
    fn unwindowed(db: &SourceDb, d: &Diagnostic) -> String {
        let labels = d
            .labels
            .iter()
            .map(|l| cs::Label::primary(l.span.file, l.span.range()).with_message(&l.message))
            .collect();
        let rendered = cs::Diagnostic::warning()
            .with_message(&d.message)
            .with_code(d.code.unwrap_or_default())
            .with_labels(labels);
        let mut buffer = termcolor::Buffer::no_color();
        term::emit_to_write_style(
            &mut buffer,
            &term::Config::default(),
            &DbFiles(db),
            &rendered,
        )
        .unwrap();
        String::from_utf8(buffer.into_inner()).unwrap()
    }

    fn long_file(db: &mut SourceDb) -> (FileId, String) {
        let long: String = (0..2000).map(|i| format!("{i},")).collect();
        let text = format!("local t = {{{long}}}\nprint(t)\nlocal u = {{{long}}}\n");
        (db.add("src/data.lua", text.clone()), text)
    }

    /// The caret line's first `^` is under `word` in the line above.
    #[track_caller]
    fn caret_under(out: &str, word: &str) {
        let lines: Vec<&str> = out.lines().collect();
        let i = lines.iter().position(|l| l.contains('^')).unwrap();
        let col = |s: &str, at: usize| s[..at].chars().count();
        let caret = col(lines[i], lines[i].find('^').unwrap());
        let above = lines[i - 1];
        assert_eq!(caret, col(above, above.find(word).unwrap()), "{out}");
    }

    #[test]
    fn long_lines_show_a_window_around_the_label() {
        let mut db = SourceDb::new();
        let (file, text) = long_file(&mut db);
        let r = Renderer::new(MessageFormat::Human, ColorChoice::Never);
        let at = text.find("1000,").unwrap();
        let d =
            Diagnostic::warning("W1003", "middle").with_label(Span::new(file, at..at + 4), "here");
        let out = r.render_to_string(&db, &d);
        assert!(out.lines().all(|l| l.chars().count() < 200), "{out}");
        assert!(
            out.contains("│ ...990,991,") && out.contains("1015,...\n"),
            "{out}"
        );
        caret_under(&out, "1000");
        // The location is the original one.
        let header = |s: &str| s.lines().take(2).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(header(&out), header(&unwindowed(&db, &d)));
        // At the start and at the end of a line, nothing is cut there.
        let d = Diagnostic::warning("W1003", "start").with_label(Span::new(file, 0..5), "here");
        let out = r.render_to_string(&db, &d);
        assert!(out.contains("│ local t = {0,1,2,"), "{out}");
        let end = text.find("\nprint").unwrap();
        let d =
            Diagnostic::warning("W1003", "end").with_label(Span::new(file, end - 1..end), "here");
        let out = r.render_to_string(&db, &d);
        assert!(out.contains(",1999,}\n"), "{out}");
        assert_eq!(header(&out), header(&unwindowed(&db, &d)));
    }

    #[test]
    fn several_lines_and_multi_line_labels_are_windowed() {
        let mut db = SourceDb::new();
        let (file, text) = long_file(&mut db);
        let r = Renderer::new(MessageFormat::Human, ColorChoice::Never);
        let (a, u) = (text.find("1000,").unwrap(), text.rfind("local u").unwrap());
        for d in [
            Diagnostic::warning("W1003", "apart")
                .with_label(Span::new(file, a..a + 4), "first")
                .with_label(Span::new(file, u..u + 7), "second"),
            Diagnostic::warning("W1003", "multi").with_label(Span::new(file, a..u + 7), "spans"),
        ] {
            let out = r.render_to_string(&db, &d);
            assert!(out.lines().all(|l| l.chars().count() < 200), "{out}");
            assert!(
                out.contains("2 │ │ print(t)") || out.contains("2 │ print(t)"),
                "{out}"
            );
        }
    }

    #[test]
    fn windows_cut_between_characters() {
        let mut db = SourceDb::new();
        let text = format!("x = \"{}\" .. y\n", "é".repeat(300));
        let file = db.add("src/u.lua", text.clone());
        let y = text.find(" y").unwrap() + 1;
        let d = Diagnostic::warning("W1001", "y").with_label(Span::new(file, y..y + 1), "here");
        let out = Renderer::new(MessageFormat::Human, ColorChoice::Never).render_to_string(&db, &d);
        assert!(out.contains("│ ...éé"), "{out}");
        caret_under(&out, "y\n".trim_end());
        let header = |s: &str| s.lines().take(2).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(header(&out), header(&unwindowed(&db, &d)));
    }

    /// Lines up to the limit render as they always did.
    #[test]
    fn short_lines_are_not_windowed() {
        let mut db = SourceDb::new();
        let line = format!("local t = {{{}}}", "1,".repeat(74));
        assert_eq!(line.len(), LONG_LINE);
        let file = db.add("src/a.lua", format!("{line}\n"));
        let d = Diagnostic::warning("W1003", "fits").with_label(Span::new(file, 6..7), "here");
        assert!(Windowed::new(&db, &d).is_none());
        let out = Renderer::new(MessageFormat::Human, ColorChoice::Never).render_to_string(&db, &d);
        assert_eq!(out, unwindowed(&db, &d));
    }

    #[test]
    fn path_only_diagnostics() {
        let db = SourceDb::new();
        let d = Diagnostic::plain_error("file not found").with_path("src/main.lua");
        let out = Renderer::new(MessageFormat::Short, ColorChoice::Never).render_to_string(&db, &d);
        assert_eq!(out, "src/main.lua: error: file not found\n");
    }
}
