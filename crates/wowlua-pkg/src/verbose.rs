//! `-v, --verbose` (spec CLI §12.1): the external commands the package
//! manager runs and the requests publishing makes are shown as they start.
//! Only commands and URLs are shown, never headers, so tokens stay hidden.

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Turns the extra lines on for the rest of the process.
pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// A status line on stderr, `     Running git …`, when verbose.
pub fn note(word: &str, message: &str) {
    if on() {
        eprintln!("{word:>12} {message}");
    }
}

/// How a command line reads: the program and its arguments, quoted where
/// they contain spaces.
pub fn command_line(command: &std::process::Command) -> String {
    let quote = |s: &std::ffi::OsStr| {
        let s = s.to_string_lossy();
        if s.is_empty() || s.contains(char::is_whitespace) {
            format!("\"{s}\"")
        } else {
            s.into_owned()
        }
    };
    std::iter::once(quote(command.get_program()))
        .chain(command.get_args().map(quote))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_lines_quote_arguments_with_spaces() {
        let mut c = std::process::Command::new("git");
        c.args(["clone", "/a b/c", ""]);
        assert_eq!(super::command_line(&c), "git clone \"/a b/c\" \"\"");
    }
}
