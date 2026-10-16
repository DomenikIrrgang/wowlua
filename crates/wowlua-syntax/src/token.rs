//! Tokens and trivia produced by the lexer.

/// The kind of a token. Contextual keywords (`class`, `import`, …) are lexed
/// as [`TokenKind::Name`] and recognized by the parser (spec §1.2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Name,
    Number,
    /// A quoted string (`'…'` or `"…"`).
    String,
    /// A long string (`[[…]]`, `[==[…]==]`).
    LongString,

    // Lua 5.1 reserved words.
    And,
    Break,
    Do,
    Else,
    Elseif,
    End,
    False,
    For,
    Function,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,

    // Lua 5.1 symbols.
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Hash,
    EqEq,
    NotEq,
    LtEq,
    GtEq,
    Lt,
    Gt,
    Eq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Colon,
    Comma,
    Dot,
    DotDot,
    Ellipsis,

    // wowlua symbols (spec §1.2.1).
    At,
    Question,
    QuestionDot,
    QuestionColon,
    QuestionQuestion,
    QuestionQuestionEq,
    ColonColon,
    Arrow,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    SlashSlash,
    SlashSlashEq,
    PercentEq,
    CaretEq,
    DotDotEq,
    Amp,
    AmpEq,
    Pipe,
    PipeEq,
    Tilde,
    Shl,
    ShlEq,
    Shr,
    ShrEq,

    // Interpolated strings: `` `a{x}b` `` is InterpStart, InterpText("a"),
    // InterpOpen, Name(x), InterpClose, InterpText("b"), InterpEnd.
    InterpStart,
    InterpText,
    InterpOpen,
    InterpClose,
    InterpEnd,

    /// A byte sequence that is not a token (reported by the lexer).
    Error,
    Eof,
}

impl TokenKind {
    /// The reserved word for `name`, if it is one.
    pub fn keyword(name: &[u8]) -> Option<TokenKind> {
        Some(match name {
            b"and" => TokenKind::And,
            b"break" => TokenKind::Break,
            b"do" => TokenKind::Do,
            b"else" => TokenKind::Else,
            b"elseif" => TokenKind::Elseif,
            b"end" => TokenKind::End,
            b"false" => TokenKind::False,
            b"for" => TokenKind::For,
            b"function" => TokenKind::Function,
            b"if" => TokenKind::If,
            b"in" => TokenKind::In,
            b"local" => TokenKind::Local,
            b"nil" => TokenKind::Nil,
            b"not" => TokenKind::Not,
            b"or" => TokenKind::Or,
            b"repeat" => TokenKind::Repeat,
            b"return" => TokenKind::Return,
            b"then" => TokenKind::Then,
            b"true" => TokenKind::True,
            b"until" => TokenKind::Until,
            b"while" => TokenKind::While,
            _ => return None,
        })
    }

    pub fn is_keyword(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            And | Break
                | Do
                | Else
                | Elseif
                | End
                | False
                | For
                | Function
                | If
                | In
                | Local
                | Nil
                | Not
                | Or
                | Repeat
                | Return
                | Then
                | True
                | Until
                | While
        )
    }

    /// A short description for diagnostics (`'end'`, `<name>`, …).
    pub fn describe(self) -> &'static str {
        use TokenKind::*;
        match self {
            Name => "<name>",
            Number => "<number>",
            String | LongString => "<string>",
            And => "'and'",
            Break => "'break'",
            Do => "'do'",
            Else => "'else'",
            Elseif => "'elseif'",
            End => "'end'",
            False => "'false'",
            For => "'for'",
            Function => "'function'",
            If => "'if'",
            In => "'in'",
            Local => "'local'",
            Nil => "'nil'",
            Not => "'not'",
            Or => "'or'",
            Repeat => "'repeat'",
            Return => "'return'",
            Then => "'then'",
            True => "'true'",
            Until => "'until'",
            While => "'while'",
            Plus => "'+'",
            Minus => "'-'",
            Star => "'*'",
            Slash => "'/'",
            Percent => "'%'",
            Caret => "'^'",
            Hash => "'#'",
            EqEq => "'=='",
            NotEq => "'~='",
            LtEq => "'<='",
            GtEq => "'>='",
            Lt => "'<'",
            Gt => "'>'",
            Eq => "'='",
            LParen => "'('",
            RParen => "')'",
            LBrace => "'{'",
            RBrace => "'}'",
            LBracket => "'['",
            RBracket => "']'",
            Semi => "';'",
            Colon => "':'",
            Comma => "','",
            Dot => "'.'",
            DotDot => "'..'",
            Ellipsis => "'...'",
            At => "'@'",
            Question => "'?'",
            QuestionDot => "'?.'",
            QuestionColon => "'?:'",
            QuestionQuestion => "'??'",
            QuestionQuestionEq => "'??='",
            ColonColon => "'::'",
            Arrow => "'->'",
            PlusEq => "'+='",
            MinusEq => "'-='",
            StarEq => "'*='",
            SlashEq => "'/='",
            SlashSlash => "'//'",
            SlashSlashEq => "'//='",
            PercentEq => "'%='",
            CaretEq => "'^='",
            DotDotEq => "'..='",
            Amp => "'&'",
            AmpEq => "'&='",
            Pipe => "'|'",
            PipeEq => "'|='",
            Tilde => "'~'",
            Shl => "'<<'",
            ShlEq => "'<<='",
            Shr => "'>>'",
            ShrEq => "'>>='",
            InterpStart | InterpEnd => "'`'",
            InterpText => "<string>",
            InterpOpen => "'{'",
            InterpClose => "'}'",
            Error => "<invalid>",
            Eof => "<eof>",
        }
    }
}

/// Line breaks and comments before a token, kept for the formatter and for
/// comment-preserving emission. Runs of spaces are not recorded (#19): no
/// reader needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriviaKind {
    /// One line break (`\n`, `\r`, `\r\n` or `\n\r`, counted like Lua 5.1).
    Newline,
    /// `-- …` up to the end of the line (the line break is not included).
    LineComment,
    /// `--[[ … ]]` / `--[==[ … ]==]`.
    BlockComment,
}

// Tokens and trivia store no line numbers: a large data addon has tens of
// millions of tokens, so every field counts. `Lexed::line_of` finds a
// position's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trivia {
    pub kind: TriviaKind,
    pub start: u32,
    pub end: u32,
}

/// A token: its kind and byte range. A token's leading trivia is the trivia
/// between the previous token's end and its start
/// ([`Lexed::trivia_of`](crate::Lexed::trivia_of)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u32,
    pub end: u32,
}

/// A file's tokens, stored by field (#19): 9 bytes per token. A data addon
/// has tens of millions of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    kinds: Vec<TokenKind>,
    starts: Vec<u32>,
    ends: Vec<u32>,
}

impl Tokens {
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Token `index`; panics past the end, like indexing a slice.
    pub fn token(&self, index: usize) -> Token {
        Token {
            kind: self.kinds[index],
            start: self.starts[index],
            end: self.ends[index],
        }
    }

    /// Token `index`, if there is one.
    pub fn get(&self, index: usize) -> Option<Token> {
        (index < self.len()).then(|| self.token(index))
    }

    /// Every token's start, in order.
    pub fn starts(&self) -> &[u32] {
        &self.starts
    }

    /// The index of the first token that starts at or after `offset` (the
    /// number of tokens if none does).
    pub fn first_from(&self, offset: u32) -> usize {
        self.starts.partition_point(|&s| s < offset)
    }

    pub fn kind(&self, index: usize) -> TokenKind {
        self.kinds[index]
    }

    pub fn start(&self, index: usize) -> u32 {
        self.starts[index]
    }

    pub fn end(&self, index: usize) -> u32 {
        self.ends[index]
    }

    pub fn first(&self) -> Option<Token> {
        self.get(0)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = Token> + '_ {
        (0..self.len()).map(|i| self.token(i))
    }

    pub fn push(&mut self, token: Token) {
        self.kinds.push(token.kind);
        self.starts.push(token.start);
        self.ends.push(token.end);
    }

    /// Replaces token `index` (the parser splits `>>` and `?:`).
    pub fn set(&mut self, index: usize, token: Token) {
        self.kinds[index] = token.kind;
        self.starts[index] = token.start;
        self.ends[index] = token.end;
    }

    pub fn insert(&mut self, index: usize, token: Token) {
        self.kinds.insert(index, token.kind);
        self.starts.insert(index, token.start);
        self.ends.insert(index, token.end);
    }

    /// Gives back the room growing left unused.
    pub fn shrink_to_fit(&mut self) {
        self.kinds.shrink_to_fit();
        self.starts.shrink_to_fit();
        self.ends.shrink_to_fit();
    }

    /// The heap the tokens take, in bytes.
    pub fn heap_bytes(&self) -> usize {
        self.kinds.capacity() * std::mem::size_of::<TokenKind>()
            + (self.starts.capacity() + self.ends.capacity()) * 4
    }
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    // One per token and per comment, line break or run of spaces.
    #[test]
    fn tokens_and_trivia_stay_small() {
        assert!(std::mem::size_of::<TokenKind>() == 1);
        assert!(std::mem::size_of::<Token>() <= 12);
        assert!(std::mem::size_of::<Trivia>() <= 12);
    }
}
