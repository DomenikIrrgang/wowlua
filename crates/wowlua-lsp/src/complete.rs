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
/// comment, a long string or a number, or no repair of the text around it
/// parses. In a quoted string, its text becomes the placeholder (the
/// strings its expected type names are completed).
pub fn prepare(text: &str, offset: usize) -> Option<Asked> {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let bytes = text.as_bytes();
    let string = match place(text, offset) {
        Place::Code => None,
        Place::String(s) => Some(s),
        Place::None => return None,
    };
    let (start, end, p) = match &string {
        // the string's text, its closing quote added if it has none yet
        Some(s) if s.closed => (s.start + 1, s.end - 1, PLACEHOLDER.to_string()),
        Some(s) => (s.start + 1, s.end, format!("{PLACEHOLDER}{}", s.quote)),
        None => {
            let mut start = offset;
            while start > 0 && is_word(bytes[start - 1]) {
                start -= 1;
            }
            let mut end = offset;
            while end < bytes.len() && is_word(bytes[end]) {
                end += 1;
            }
            if bytes.get(start).is_some_and(u8::is_ascii_digit) {
                return None;
            }
            (start, end, PLACEHOLDER.to_string())
        }
    };
    let mut db = SourceDb::new();
    let file = db.add("complete", String::new());
    let prefix = text[start..offset.max(start)].to_string();
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
    // a decorator with nothing after it yet: a function to put it on
    if string.is_none() && before.ends_with('@') {
        candidates.push(format!(
            "{before}{p}{closers}\nlocal function _() end{rest}"
        ));
    }
    // the line alone, as a statement of its own: what precedes the
    // placeholder from where its expression starts
    // (the block a header opens stays open, for its `end`)
    if string.is_none() {
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
    }
    let text = candidates
        .into_iter()
        .find(|c| wowlua_syntax::parses(c.as_bytes(), file))?;
    Some(Asked { prefix, text })
}

/// Where the cursor is: in code, in a quoted string, or where nothing is
/// completed (a comment, a long string).
enum Place {
    Code,
    String(Quoted),
    None,
}

/// A quoted string: where it starts and ends, its quote, and whether it has
/// its closing quote (one being typed runs to the end of its line).
struct Quoted {
    start: usize,
    end: usize,
    quote: char,
    closed: bool,
}

fn place(text: &str, offset: usize) -> Place {
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
    if in_comment {
        return Place::None;
    }
    for t in lexed.tokens.iter() {
        if !matches!(t.kind, TokenKind::String | TokenKind::LongString) {
            continue;
        }
        let s = &text.as_bytes()[t.start as usize..t.end as usize];
        let long = t.kind == TokenKind::LongString;
        let closed = long || (s.len() >= 2 && s.last() == s.first() && s[s.len() - 2] != b'\\');
        if !(t.start < at && (at < t.end || (at == t.end && !closed))) {
            continue;
        }
        // (a string with escapes or line breaks is no name)
        if long || s.contains(&b'\\') {
            return Place::None;
        }
        return Place::String(Quoted {
            start: t.start as usize,
            end: t.end as usize,
            quote: s[0] as char,
            closed,
        });
    }
    Place::Code
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

/// The call whose argument list holds the cursor (signature help): the
/// argument it is in (from 0), and the end of the called name (its last
/// name: `GetSpellInfo` of `C_Spell.GetSpellInfo`) with that name.
#[derive(Debug, PartialEq)]
pub struct CallSite {
    pub active: usize,
    pub name: String,
    pub name_end: usize,
}

/// The innermost call around byte `offset` whose `(` is open there; none
/// in a function's body inside the arguments, a table's braces count as
/// part of the argument they are in.
pub fn call_at(text: &str, offset: usize) -> Option<CallSite> {
    use TokenKind as T;
    let mut db = SourceDb::new();
    let file = db.add("signature", String::new());
    let lexed = wowlua_syntax::lexer::lex(text.as_bytes(), file);
    let tokens: Vec<_> = lexed
        .tokens
        .iter()
        .filter(|t| {
            (t.end as usize) <= offset || (t.start as usize) < offset && matches!(t.kind, T::String)
        })
        .collect();
    let mut depth = 0usize;
    let mut active = 0usize;
    for (i, t) in tokens.iter().enumerate().rev() {
        match t.kind {
            T::RParen | T::RBracket | T::RBrace => depth += 1,
            T::LBracket | T::LBrace if depth > 0 => depth -= 1,
            // an open table or index: its commas are another list's
            T::LBracket | T::LBrace => active = 0,
            T::LParen if depth > 0 => depth -= 1,
            T::LParen => {
                let before = i.checked_sub(1).map(|j| tokens[j]);
                match before.map(|b| b.kind) {
                    Some(T::Name) => {
                        let b = before.expect("a token");
                        return Some(CallSite {
                            active,
                            name: text[b.start as usize..b.end as usize].to_string(),
                            name_end: b.end as usize,
                        });
                    }
                    Some(T::RParen | T::RBracket) => {
                        return Some(CallSite {
                            active,
                            name: "function".into(),
                            name_end: before.expect("a token").end as usize,
                        });
                    }
                    // `function(` of a function's parameters, or a grouping
                    // parenthesis: not a call
                    Some(T::Function) => return None,
                    _ => active = 0,
                }
            }
            T::Comma if depth == 0 => active += 1,
            // a statement starts here: the cursor is in no call
            T::Function
            | T::Do
            | T::Then
            | T::Else
            | T::Elseif
            | T::Repeat
            | T::Until
            | T::End
            | T::Local
            | T::Return
            | T::While
            | T::For
            | T::If
            | T::Semi
                if depth == 0 =>
            {
                return None;
            }
            _ => {}
        }
    }
    None
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
        // a decorator: above a function, or with nothing after it yet
        assert_eq!(
            checked("@gl|\nfunction f() end\n"),
            "@@\nfunction f() end\n"
        );
        assert_eq!(checked("@|"), "@@\nlocal function _() end");
    }

    #[test]
    fn no_completion_in_comments_long_strings_and_numbers() {
        assert!(at("-- self.|\n").is_none());
        assert!(at("--[[ x.| ]]\n").is_none());
        assert!(at("local s = [[a.|]]\n").is_none());
        assert!(at("local n = 1.|\n").is_none());
        // after a closed string: completion again
        assert!(at("local s = \"a\" .. |\n").is_some());
    }

    #[test]
    fn the_call_around_the_cursor_and_its_argument() {
        let site = |t: &str| {
            let offset = t.find('|').expect("a cursor");
            call_at(&t.replacen('|', "", 1), offset).map(|c| (c.name, c.active))
        };
        assert_eq!(
            site("local x = C_Spell.GetSpellInfo(|)"),
            Some(("GetSpellInfo".into(), 0))
        );
        assert_eq!(site("f(a, |"), Some(("f".into(), 1)));
        assert_eq!(site("f(a, g(b, c|), d)"), Some(("g".into(), 1)));
        assert_eq!(site("f(a, { 1, 2, | })"), Some(("f".into(), 1)));
        assert_eq!(site("f(\"a, b\", |)"), Some(("f".into(), 1)));
        assert_eq!(
            site("frame:SetPoint(\"TOP\", |"),
            Some(("SetPoint".into(), 1))
        );
        // in a function passed as an argument, a grouping parenthesis
        assert_eq!(site("f(function() x = | end)"), None);
        assert_eq!(site("local y = (a + |)"), None);
        assert_eq!(site("local z = |"), None);
    }

    #[test]
    fn a_strings_text_becomes_the_placeholder() {
        let a = at("f:RegisterEvent(\"UNIT_|HEALTH\")\n").unwrap();
        assert_eq!(a.prefix, "UNIT_");
        assert_eq!(a.text.replace(PLACEHOLDER, "@"), "f:RegisterEvent(\"@\")\n");
        // still being typed: closed
        assert_eq!(
            checked("f:RegisterEvent(\"UNIT_|\n"),
            "f:RegisterEvent(\"@\")\n"
        );
        assert_eq!(checked("if event == '|\n"), "if event == '@' then end\n");
        assert_eq!(
            checked("local m: \"a\" = \"|\"\n"),
            "local m: \"a\" = \"@\"\n"
        );
    }
}
