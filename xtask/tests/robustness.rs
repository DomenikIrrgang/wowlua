//! Randomized robustness tests (spec compiler §13.1: the compiler never
//! panics on any input). Deterministic, so failures reproduce. For every
//! input: parsing never panics; code that lowers emits Lua that parses; the
//! formatter never changes a program's meaning.

use wowlua_luair::{EmitOptions, LineMode};
use wowlua_span::SourceDb;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn exercise(src: &[u8]) {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", String::from_utf8_lossy(src).into_owned());
    let parsed = wowlua_syntax::parse(src, file);
    let _ = wowlua_syntax::features::features(parsed.view());
    if parsed.has_errors() {
        return;
    }
    if let Ok(chunk) = wowlua_lower::lower(&parsed, src) {
        for line_mode in [LineMode::Preserve, LineMode::Statement, LineMode::Pretty] {
            let out = wowlua_luair::emit(
                &chunk,
                &EmitOptions {
                    line_mode,
                    ..EmitOptions::default()
                },
            );
            let again = wowlua_syntax::parse(&out.text, file);
            assert!(
                !again.has_errors(),
                "emitted code does not parse\ninput: {:?}\noutput: {:?}\n{:?}",
                String::from_utf8_lossy(src),
                String::from_utf8_lossy(&out.text),
                again.diagnostics
            );
        }
    }
    if let Err(wowlua_fmt::Error::Unstable(reason)) =
        wowlua_fmt::format(src, file, &wowlua_fmt::Config::default())
    {
        panic!(
            "formatter: {reason}\ninput: {:?}",
            String::from_utf8_lossy(src)
        );
    }
}

#[test]
fn random_bytes() {
    let alphabet: Vec<u8> =
        b"abcxyz_019 \t\n\r.,;:=<>~+-*/%^#&|?@`'\"[]{}()\\!$\xc3\xa9\xe9".to_vec();
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..30_000 {
        let len = rng.below(60);
        let src: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        exercise(&src);
    }
}

const VOCAB: &[&str] = &[
    "local",
    "function",
    "end",
    "if",
    "then",
    "else",
    "elseif",
    "while",
    "do",
    "for",
    "in",
    "repeat",
    "until",
    "return",
    "break",
    "and",
    "or",
    "not",
    "nil",
    "true",
    "false",
    "a",
    "b",
    "f",
    "x",
    "1",
    "0x1F",
    "1_000",
    "0b101",
    "2.5",
    "'s'",
    "\"d\"",
    "[[l]]",
    "[==[m]==]",
    "...",
    "=",
    "==",
    "~=",
    "<",
    "<=",
    ">",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "//",
    "%",
    "^",
    "#",
    "..",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    ".",
    ":",
    "::",
    "?",
    "?.",
    "?:",
    "??",
    "??=",
    "+=",
    "..=",
    "&",
    "|",
    "~",
    "<<",
    ">>",
    "->",
    "@",
    "`a{x}b`",
    "`",
    "class",
    "interface",
    "enum",
    "type",
    "const",
    "declare",
    "import",
    "export",
    "from",
    "as",
    "extends",
    "continue",
    "number",
    "string",
    "--c\n",
    "--[[b]]",
    "\n",
];

#[test]
fn random_token_soup() {
    let mut rng = Rng(0x0F1E_2D3C_4B5A_6978);
    for _ in 0..30_000 {
        let len = 1 + rng.below(16);
        let src: Vec<&str> = (0..len).map(|_| VOCAB[rng.below(VOCAB.len())]).collect();
        exercise(src.join(" ").as_bytes());
    }
}

#[test]
fn mutated_samples() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let samples: Vec<Vec<u8>> = std::fs::read_dir(root.join("tests/corpus-sample"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "lua"))
        .map(|e| std::fs::read(e.path()).unwrap())
        .collect();
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..3_000 {
        let mut src = samples[rng.below(samples.len())].clone();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(src.len() + 1);
            match rng.below(3) {
                0 if at < src.len() => {
                    let end = (at + 1 + rng.below(8)).min(src.len());
                    src.drain(at..end);
                }
                1 => {
                    let token = VOCAB[rng.below(VOCAB.len())].as_bytes();
                    src.splice(at..at, token.iter().copied());
                }
                _ if at < src.len() => src[at] = b" =(){}[]\"'-\n"[rng.below(12)],
                _ => {}
            }
        }
        exercise(&src);
    }
}
