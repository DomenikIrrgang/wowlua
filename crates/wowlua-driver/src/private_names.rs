//! The class half of `rename-private` (spec optimization §15.4.1): private
//! and protected methods get short names, when that provably changes
//! nothing. A method name is renamed only if every occurrence of it in the
//! package (as a name or inside a string), in its declarations, the WoW API
//! declarations, its XML and its embedded libraries is a definition of such
//! a method or an access the checker resolved to one. The new names occur
//! nowhere in any of them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use wowlua_luair::ir::Pos;
use wowlua_span::{FileId, Span};
use wowlua_syntax::{Parse, token::TokenKind};

/// A parsed file and its bytes.
pub struct Parsed<'a> {
    pub file: FileId,
    pub parse: &'a Parse,
    pub bytes: &'a [u8],
}

/// Old name → new name, and per file the positions to rename.
#[derive(Default)]
pub struct Plan {
    pub at: HashMap<FileId, HashMap<Pos, (String, String)>>,
}

fn words(text: &[u8], mut f: impl FnMut(&[u8])) {
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < text.len() && (text[i].is_ascii_alphanumeric() || text[i] == b'_') {
                i += 1;
            }
            f(&text[start..i]);
        } else {
            i += 1;
        }
    }
}

/// Plans the renames. `recorded`: the checker's definition and access spans;
/// `foreign`: other text the names must not occur in (XML, libraries).
pub fn plan(recorded: &HashSet<Span>, parsed: &[Parsed], foreign: &[&[u8]]) -> Plan {
    let by_file: HashMap<FileId, &Parsed> = parsed.iter().map(|p| (p.file, p)).collect();
    let text_of = |span: &Span| -> Option<String> {
        let p = by_file.get(&span.file)?;
        let bytes = p.bytes.get(span.start as usize..span.end as usize)?;
        String::from_utf8(bytes.to_vec()).ok()
    };
    // Candidate names and how often they are used.
    let mut candidates: BTreeMap<String, usize> = BTreeMap::new();
    for span in recorded {
        if let Some(name) = text_of(span) {
            *candidates.entry(name).or_default() += 1;
        }
    }
    if candidates.is_empty() {
        return Plan::default();
    }
    let mut unsafe_names: HashSet<String> = HashSet::new();
    let mut taken: HashSet<Vec<u8>> = HashSet::new();
    for p in parsed {
        for token in &p.parse.lexed.tokens {
            let Some(bytes) = p.bytes.get(token.start as usize..token.end as usize) else {
                continue;
            };
            match token.kind {
                TokenKind::String | TokenKind::LongString => {
                    let content = wowlua_luair::minify::literals::decode_string(bytes)
                        .unwrap_or_else(|| bytes.to_vec());
                    words(&content, |w| {
                        taken.insert(w.to_vec());
                    });
                    if let Ok(s) = std::str::from_utf8(&content)
                        && candidates.contains_key(s)
                    {
                        unsafe_names.insert(s.to_string());
                    }
                }
                _ => {
                    taken.insert(bytes.to_vec());
                    if let Ok(s) = std::str::from_utf8(bytes)
                        && candidates.contains_key(s)
                        && !recorded.contains(&Span {
                            file: p.file,
                            start: token.start,
                            end: token.end,
                        })
                    {
                        unsafe_names.insert(s.to_string());
                    }
                }
            }
        }
    }
    for text in foreign {
        words(text, |w| {
            if let Ok(s) = std::str::from_utf8(w)
                && candidates.contains_key(s)
            {
                unsafe_names.insert(s.to_string());
            }
            taken.insert(w.to_vec());
        });
    }
    // The most used names get the shortest new ones.
    let mut safe: Vec<(&String, &usize)> = candidates
        .iter()
        .filter(|(n, _)| !unsafe_names.contains(*n))
        .collect();
    safe.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut fresh = (0..)
        .map(wowlua_luair::minify::rename::short_name)
        .filter(|n| !taken.contains(n.as_bytes()) && !is_keyword(n));
    let renames: HashMap<&String, String> = safe
        .into_iter()
        .map(|(n, _)| (n, fresh.next().expect("unbounded")))
        .filter(|(old, new)| new.len() < old.len())
        .collect();
    let mut plan = Plan::default();
    for span in recorded {
        let Some(old) = text_of(span) else { continue };
        let Some(new) = renames.get(&old) else {
            continue;
        };
        let lexed = &by_file[&span.file].parse.lexed;
        let pos = Pos::new(lexed.line_of(span.start), lexed.column_of(span.start));
        plan.at
            .entry(span.file)
            .or_default()
            .insert(pos, (old, new.clone()));
    }
    plan
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

/// Drops the names of which some position is missing from the lowered code
/// (`found`: the positions each file's code has).
pub fn verify(plan: &mut Plan, found: &HashMap<FileId, Vec<Pos>>) {
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for (file, at) in &plan.at {
        let have: HashSet<&Pos> = found.get(file).into_iter().flatten().collect();
        for (pos, (old, _)) in at {
            if !have.contains(pos) {
                missing.insert(old.clone());
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    for at in plan.at.values_mut() {
        at.retain(|_, (old, _)| !missing.contains(old));
    }
}
