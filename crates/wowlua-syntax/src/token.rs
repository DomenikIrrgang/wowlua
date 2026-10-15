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

/// Whitespace and comments before a token. Kept for the formatter and for
/// comment-preserving emission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriviaKind {
    Whitespace,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u32,
    pub end: u32,
    /// End of the token's leading trivia in `Lexed::trivia`. It starts where
    /// the previous token's ends ([`Lexed::trivia_of`](crate::Lexed::trivia_of)).
    pub trivia_end: u32,
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;

    // One per token and per comment, line break or run of spaces.
    #[test]
    fn tokens_and_trivia_stay_small() {
        assert!(std::mem::size_of::<Token>() <= 16);
        assert!(std::mem::size_of::<Trivia>() <= 12);
    }
}
