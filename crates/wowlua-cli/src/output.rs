//! Terminal output: cargo-style status lines and diagnostic rendering.

use crate::{Color, Format, GlobalArgs};
use std::io::Write;
use termcolor::{ColorChoice, ColorSpec, StandardStream, WriteColor};
use wowlua_diagnostics::{ColorChoice as DiagColor, MessageFormat, Renderer, Severity};
use wowlua_driver::Session;

pub struct Output {
    renderer: Renderer,
    color: ColorChoice,
    quiet: bool,
    deny_warnings: bool,
}

impl Output {
    pub fn new(global: &GlobalArgs) -> Self {
        let format = match global.message_format {
            Format::Human => MessageFormat::Human,
            Format::Short => MessageFormat::Short,
            Format::Json => MessageFormat::Json,
            Format::Github => MessageFormat::Github,
        };
        let (diag_color, color) = match global.color {
            Color::Always => (DiagColor::Always, ColorChoice::Always),
            Color::Never => (DiagColor::Never, ColorChoice::Never),
            Color::Auto if std::env::var_os("NO_COLOR").is_some() => {
                (DiagColor::Never, ColorChoice::Never)
            }
            Color::Auto if std::io::IsTerminal::is_terminal(&std::io::stderr()) => {
                (DiagColor::Auto, ColorChoice::Auto)
            }
            Color::Auto => (DiagColor::Never, ColorChoice::Never),
        };
        Output {
            renderer: Renderer::new(format, diag_color),
            color,
            quiet: global.quiet || matches!(format, MessageFormat::Json),
            deny_warnings: global.deny_warnings,
        }
    }

    /// A right-aligned green status word, like cargo: `   Compiling MyAddon`.
    pub fn status(&self, word: &str, message: impl AsRef<str>) {
        if self.quiet {
            return;
        }
        let mut stream = StandardStream::stderr(self.color);
        let _ = stream.set_color(
            ColorSpec::new()
                .set_fg(Some(termcolor::Color::Green))
                .set_bold(true),
        );
        let _ = write!(stream, "{word:>12}");
        let _ = stream.reset();
        let _ = writeln!(stream, " {}", message.as_ref());
    }

    /// A status sink for the driver's progress lines.
    pub fn status_sink(&self) -> wowlua_driver::Status {
        let (color, quiet) = (self.color, self.quiet);
        wowlua_driver::Status(std::rc::Rc::new(move |word: &str, message: &str| {
            if quiet {
                return;
            }
            let mut stream = StandardStream::stderr(color);
            let _ = stream.set_color(
                ColorSpec::new()
                    .set_fg(Some(termcolor::Color::Green))
                    .set_bold(true),
            );
            let _ = write!(stream, "{word:>12}");
            let _ = stream.reset();
            let _ = writeln!(stream, " {message}");
        }))
    }

    /// Prints a line to stdout (command results such as TOC text).
    pub fn print(&self, text: impl AsRef<str>) {
        print!("{}", text.as_ref());
    }

    /// Renders and drains the session's diagnostics. Returns `true` if the
    /// command may continue (no errors, and no warnings with
    /// `--deny-warnings`).
    pub fn flush(&self, session: &mut Session) -> bool {
        let diagnostics = session.take_diagnostics();
        let fail = diagnostics.should_fail(self.deny_warnings);
        let items: Vec<_> = diagnostics
            .into_vec()
            .into_iter()
            .filter(|d| !(self.quiet && d.severity == Severity::Note))
            .collect();
        let _ = self.renderer.emit_stderr(&session.db, &items);
        !fail
    }
}
