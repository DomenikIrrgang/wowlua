//! Completion (spec §16.2): the word at the cursor becomes the checker's
//! placeholder, in a text that parses (the code around a cursor is often
//! unfinished: `self.`, `if frame:`), and a check of that text finds what
//! could stand there.

use wowlua_sema::check::complete::PLACEHOLDER;
use wowlua_span::SourceDb;
use wowlua_syntax::token::{TokenKind, TriviaKind};

/// A completion to check.
#[derive(Debug, PartialEq)]
pub struct Asked {
    /// The word before the cursor.
    pub prefix: String,
    /// The text to check: the word at the cursor is the placeholder.
    pub text: String,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The completion at byte `offset` of `text`, unless the cursor is in a
/// comment, a string or a number, or no repair of the text around it
/// parses.
pub fn prepare(text: &str, offset: usize) -> Option<Asked> {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let bytes = text.as_bytes();
    let mut start = offset;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if bytes.get(start).is_some_and(u8::is_ascii_digit) || quoted(text, offset) {
        return None;
    }
    let mut db = SourceDb::new();
    let file = db.add("complete", String::new());
    let prefix = text[start..offset].to_string();
    let before = &text[..start];
    let after = &text[end..];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line_end = after.find('\n').unwrap_or(after.len());
    // (from the line break on)
    let rest = &after[line_end..];
    let closers = closers(&text[line_start..start]);
    let lead = text[line_start..start].trim_start();
    let indent = &text[line_start..line_start + (start - line_start - lead.len())];
    // a block's header, unfinished (`if self.`): finished
    let header = match lead.split(|c: char| !is_word(c as u8)).next() {
        Some("if") => " then end",
        Some("elseif") => " then",
        Some("while" | "for") => " do end",
        _ => "",
    };
    let p = PLACEHOLDER;
    let mut candidates = vec![
        format!("{before}{p}{after}"),
        // a call statement (`frame:Show()`)
        format!("{before}{p}(){after}"),
        // the rest of the line left out, its brackets closed
        format!("{before}{p}{closers}{rest}"),
        format!("{before}{p}(){closers}{rest}"),
    ];
    if !header.is_empty() {
        candidates.push(format!("{before}{p}{closers}{header}{rest}"));
        candidates.push(format!("{before}{p}(){closers}{header}{rest}"));
    }
    // the line alone, as a statement of its own: what precedes the
    // placeholder from where its expression starts
    // (the block a header opens stays open, for its `end`)
    let receiver = receiver(&text[line_start..start]);
    let call = if receiver.ends_with(':') { "()" } else { "" };
    let opens = match header {
        " then end" | " do end" => "do ",
        " then" => "elseif _ then ",
        _ => "",
    };
    candidates.push(format!(
        "{}{indent}{opens}local _ = {receiver}{p}{call}{rest}",
        &text[..line_start]
    ));
    let text = candidates
        .into_iter()
        .find(|c| wowlua_syntax::parses(c.as_bytes(), file))?;
    Some(Asked { prefix, text })
}

/// Whether byte `offset` is inside a comment or a quoted string.
fn quoted(text: &str, offset: usize) -> bool {
    let mut db = SourceDb::new();
    let file = db.add("complete", String::new());
    let lexed = wowlua_syntax::lexer::lex_with(
        text.as_bytes(),
        file,
        wowlua_syntax::lexer::LexOptions { trivia: true },
    );
    let at = offset as u32;
    let in_comment = lexed.trivia.iter().any(|t| match t.kind {
        TriviaKind::LineComment => t.start < at && at <= t.end,
        TriviaKind::BlockComment => t.start < at && at < t.end,
        TriviaKind::Newline => false,
    });
    let in_string = lexed.tokens.iter().any(|t| {
        if !matches!(t.kind, TokenKind::String | TokenKind::LongString) {
            return false;
        }
        let s = &text.as_bytes()[t.start as usize..t.end as usize];
        // (an unfinished string runs to the end of its line)
        let closed = s.len() >= 2 && s.last() == s.first() || t.kind == TokenKind::LongString;
        t.start < at && (at < t.end || (at == t.end && !closed))
    });
    in_comment || in_string
}

/// What closes the brackets `line` opens and leaves open.
fn closers(line: &str) -> String {
    let mut open = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in line.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => open.push(')'),
            '[' => open.push(']'),
            '{' => open.push('}'),
            ')' | ']' | '}' => {
                open.pop();
            }
            _ => {}
        }
    }
    open.iter().rev().collect()
}

/// The end of `line` from where the expression whose member the cursor is
/// starts (`self.bar.` of `x = self.bar.`): a name, then members, calls
/// and indexes; empty when the cursor is no member's.
fn receiver(line: &str) -> &str {
    let b = line.as_bytes();
    if !matches!(b.last(), Some(b'.' | b':')) {
        return "";
    }
    let mut i = b.len();
    while i > 0 {
        let c = b[i - 1];
        if is_word(c) || c == b'.' || c == b':' {
            i -= 1;
        } else if c == b')' || c == b']' {
            // back to the bracket that opens it
            let mut depth = 0;
            while i > 0 {
                match b[i - 1] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    _ => {}
                }
                i -= 1;
                if depth == 0 {
                    break;
                }
            }
        } else {
            break;
        }
    }
    &line[i..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Option<Asked> {
        let offset = text.find('|').expect("a cursor");
        prepare(&text.replacen('|', "", 1), offset)
    }

    fn checked(text: &str) -> String {
        at(text).expect("completes").text.replace(PLACEHOLDER, "@")
    }

    #[test]
    fn the_word_at_the_cursor_becomes_the_placeholder() {
        let a = at("local x = sel|f.y\n").unwrap();
        assert_eq!(a.prefix, "sel");
        assert_eq!(a.text.replace(PLACEHOLDER, "@"), "local x = @.y\n");
        assert_eq!(
            checked("local x = foo.|\nprint(1)\n"),
            "local x = foo.@\nprint(1)\n"
        );
    }

    #[test]
    fn unfinished_code_is_repaired_until_it_parses() {
        // a statement: a call
        assert_eq!(checked("self.|\n"), "self.@()\n");
        assert_eq!(checked("frame:Se|\n"), "frame:@()\n");
        // open brackets closed, the rest of the line left out
        assert_eq!(
            checked("print(foo.|\nlocal y = 1\n"),
            "print(foo.@)\nlocal y = 1\n"
        );
        // a block's header finished
        assert_eq!(checked("if self.|\n"), "if self.@ then end\n");
        // a header whose block follows: the line alone
        assert_eq!(
            checked("function f()\n    if self.a.|\n        x()\n    end\nend\n"),
            "function f()\n    do local _ = self.a.@\n        x()\n    end\nend\n"
        );
        // a type
        assert_eq!(checked("local x: Fr|\n"), "local x: @\n");
    }

    #[test]
    fn no_completion_in_comments_strings_and_numbers() {
        assert!(at("-- self.|\n").is_none());
        assert!(at("local s = \"a.|\"\n").is_none());
        assert!(at("local s = \"a.|\n").is_none());
        assert!(at("--[[ x.| ]]\n").is_none());
        assert!(at("local n = 1.|\n").is_none());
        // after a closed string: completion again
        assert!(at("local s = \"a\" .. |\n").is_some());
    }
}
