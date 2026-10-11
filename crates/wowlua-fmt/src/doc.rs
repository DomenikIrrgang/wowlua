//! A Wadler-style document printer: groups print flat when they fit the
//! line width, otherwise their line breaks become newlines.

#[derive(Clone, Debug)]
pub enum Doc {
    /// Source text is bytes: Lua does not require UTF-8.
    Text(Vec<u8>),
    /// A space when flat, a newline when broken.
    Line,
    /// Nothing when flat, a newline when broken.
    SoftLine,
    /// Always a newline; breaks every enclosing group.
    HardLine,
    /// A newline unless the line is still empty (comments that stand on
    /// their own line); breaks every enclosing group.
    FreshLine,
    Indent(Box<Doc>),
    Group(Box<Doc>, bool),
    Concat(Vec<Doc>),
    /// (broken, flat)
    IfBreak(Box<Doc>, Box<Doc>),
    /// Text deferred to the end of the line (trailing comments).
    LineSuffix(Vec<u8>),
    /// Breaks every enclosing group.
    BreakParent,
}

impl Doc {
    pub fn text(s: impl AsRef<[u8]>) -> Doc {
        Doc::Text(s.as_ref().to_vec())
    }

    pub fn concat(parts: Vec<Doc>) -> Doc {
        Doc::Concat(parts)
    }

    pub fn indent(doc: Doc) -> Doc {
        Doc::Indent(Box::new(doc))
    }

    pub fn group(doc: Doc) -> Doc {
        let hard = doc.has_hard_break();
        Doc::Group(Box::new(doc), hard)
    }

    pub fn if_break(broken: Doc, flat: Doc) -> Doc {
        Doc::IfBreak(Box::new(broken), Box::new(flat))
    }

    /// Contains a forced break outside nested groups (whose flag is cached).
    fn has_hard_break(&self) -> bool {
        match self {
            Doc::HardLine | Doc::FreshLine | Doc::BreakParent => true,
            Doc::Group(_, hard) => *hard,
            Doc::Indent(d) => d.has_hard_break(),
            Doc::Concat(parts) => parts.iter().any(Doc::has_hard_break),
            Doc::IfBreak(b, _) => b.has_hard_break(),
            Doc::Text(_) | Doc::Line | Doc::SoftLine | Doc::LineSuffix(_) => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// Prints `doc` within `width` columns; `indent` is one indentation unit,
/// counted as `indent_width` columns.
/// Display width of UTF-8 (or other) bytes: continuation bytes do not count.
fn width_of(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

pub fn print(doc: &Doc, width: usize, indent: &str, indent_width: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    let mut suffix: Vec<Vec<u8>> = Vec::new();
    let mut pending_indent: Option<usize> = None;
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, doc)];
    while let Some((level, mode, d)) = stack.pop() {
        match d {
            Doc::Text(s) => {
                if s.is_empty() {
                    continue;
                }
                if let Some(level) = pending_indent.take() {
                    for _ in 0..level {
                        out.extend_from_slice(indent.as_bytes());
                    }
                    pos = level * indent_width;
                }
                out.extend_from_slice(s);
                match s.iter().rposition(|&c| c == b'\n') {
                    Some(i) => pos = width_of(&s[i + 1..]),
                    None => pos += width_of(s),
                }
            }
            Doc::Line | Doc::SoftLine if mode == Mode::Flat => {
                if matches!(d, Doc::Line) {
                    out.push(b' ');
                    pos += 1;
                }
            }
            Doc::FreshLine if pending_indent.is_some() || out.is_empty() => {}
            Doc::Line | Doc::SoftLine | Doc::HardLine | Doc::FreshLine => {
                for s in suffix.drain(..) {
                    out.extend_from_slice(&s);
                }
                while out.ends_with(b" ") || out.ends_with(b"\t") {
                    out.pop();
                }
                out.push(b'\n');
                pending_indent = Some(level);
                pos = level * indent_width;
            }
            Doc::Indent(inner) => stack.push((level + 1, mode, inner)),
            Doc::Group(inner, hard) => {
                let m = if *hard {
                    Mode::Break
                } else if mode == Mode::Flat || fits(inner, width.saturating_sub(pos), &stack) {
                    Mode::Flat
                } else {
                    Mode::Break
                };
                stack.push((level, m, inner));
            }
            Doc::Concat(parts) => {
                for p in parts.iter().rev() {
                    stack.push((level, mode, p));
                }
            }
            Doc::IfBreak(broken, flat) => {
                stack.push((level, mode, if mode == Mode::Break { broken } else { flat }));
            }
            Doc::LineSuffix(s) => suffix.push(s.clone()),
            Doc::BreakParent => {}
        }
    }
    for s in suffix.drain(..) {
        out.extend_from_slice(&s);
    }
    out
}

/// Whether `doc` printed flat fits in `remaining` columns, followed by the
/// rest of the current line.
fn fits(doc: &Doc, remaining: usize, rest: &[(usize, Mode, &Doc)]) -> bool {
    let mut width = remaining as isize;
    let mut stack: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
    let mut rest_index = rest.len();
    loop {
        let Some((mode, d)) = stack.pop() else {
            // Continue with what follows the group, up to its line break.
            if rest_index == 0 {
                return true;
            }
            rest_index -= 1;
            let (_, mode, d) = rest[rest_index];
            stack.push((mode, d));
            continue;
        };
        match d {
            Doc::Text(s) => {
                // Multi-line text (long strings): only the first line counts.
                let first = s.split(|&c| c == b'\n').next().unwrap_or_default();
                width -= width_of(first) as isize;
                if s.contains(&b'\n') {
                    return width >= 0;
                }
            }
            Doc::Line if mode == Mode::Flat => width -= 1,
            Doc::SoftLine if mode == Mode::Flat => {}
            Doc::Line | Doc::SoftLine | Doc::HardLine | Doc::FreshLine => return width >= 0,
            Doc::Indent(inner) => stack.push((mode, inner)),
            Doc::Group(inner, hard) => {
                if *hard && mode == Mode::Flat {
                    return false;
                }
                stack.push((mode, inner));
            }
            Doc::Concat(parts) => {
                for p in parts.iter().rev() {
                    stack.push((mode, p));
                }
            }
            Doc::IfBreak(broken, flat) => {
                stack.push((mode, if mode == Mode::Break { broken } else { flat }));
            }
            Doc::LineSuffix(_) => {}
            Doc::BreakParent => {
                if mode == Mode::Flat {
                    return false;
                }
            }
        }
        if width < 0 {
            return false;
        }
    }
}
