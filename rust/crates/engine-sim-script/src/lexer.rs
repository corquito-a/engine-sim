//! Lexical analysis for `.mr` scripts.
//!
//! This is a faithful re-implementation of `piranha/flex-bison/scanner.l`. The
//! rules, their order and flex's longest-match semantics all matter: the
//! original scanner resolves `importx` to a label (not the `import` keyword)
//! and `1.5` to a float (not `1` followed by `.` `5`), and a hand written lexer
//! only reproduces that if it mimics maximal munch.

/// Token kinds. Byte-sized punctuation tokens keep the same spelling the
/// grammar uses ('(' , ')' , ...) so parser code reads the same as
/// `specification.y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Import,
    As,
    Input,
    Output,
    Modify,
    Toggle,
    Alias,
    Inline,
    Auto,
    Node,
    Public,
    Private,
    Operator,
    Module,
    Decorator,
    Bool,
    Label,
    Int,
    Float,
    Str,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Colon,
    Semi,
    Comma,
    Caret,
    Dot,
    Star,
    Slash,
    Plus,
    Minus,
    Eq,
    BuiltinPointer,
    NamespacePointer,
    /// `.` catch-all: the byte has no rule in `scanner.l`.
    Unrecognized,
    End,
}

/// Literal payload of a token, mirroring the `variant` semantic values used by
/// the bison parser.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    None,
    Int(i32),
    Float(f64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Kind,
    /// Raw source text of the token (as `yytext` would hold it).
    pub text: String,
    pub literal: Literal,
    /// 1-based line of the first character.
    pub line: u32,
    /// 1-based column of the first character, tabs counted as four columns.
    pub col: u32,
}

impl Token {
    pub fn is(&self, kind: Kind) -> bool {
        self.kind == kind
    }
}

const KEYWORDS: &[(&str, Kind)] = &[
    ("import", Kind::Import),
    ("as", Kind::As),
    ("input", Kind::Input),
    ("output", Kind::Output),
    ("modify", Kind::Modify),
    ("toggle", Kind::Toggle),
    ("alias", Kind::Alias),
    ("inline", Kind::Inline),
    ("auto", Kind::Auto),
    ("node", Kind::Node),
    ("public", Kind::Public),
    ("private", Kind::Private),
    ("operator", Kind::Operator),
    ("module", Kind::Module),
    ("true", Kind::Bool),
    ("false", Kind::Bool),
];

fn is_label_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_label_char(c: u8) -> bool {
    is_label_start(c) || c.is_ascii_digit()
}

/// Byte-oriented scanner over one source file.
pub struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Scanner<'a> {
    pub fn new(src: &'a str) -> Self {
        Scanner { src: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos + n).copied()
    }

    /// Mirrors `Scanner::countChar`.
    fn advance_cols(&mut self, c: u8) {
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        }
        else if c == b'\t' {
            self.col += 4;
        }
        else {
            self.col += 1;
        }
    }

    fn bump(&mut self) -> u8 {
        let c = self.src[self.pos];
        self.pos += 1;
        self.advance_cols(c);
        c
    }

    /// Skips a `/* ... */` comment, mirroring `Scanner::comment` (an
    /// unterminated comment is a lexer error there; here it just runs to EOF
    /// and the resulting `End` token fails to parse).
    fn skip_block_comment(&mut self) {
        let mut prev = 0u8;
        while let Some(c) = self.peek() {
            self.bump();
            if c == b'/' && prev == b'*' {
                return;
            }
            prev = c;
        }
    }

    fn token(&mut self, kind: Kind, line: u32, col: u32, start: usize) -> Token {
        Token {
            kind,
            text: String::from_utf8_lossy(&self.src[start..self.pos]).into_owned(),
            literal: Literal::None,
            line,
            col,
        }
    }

    /// Reads the next token. Returns `Kind::End` at end of input, like
    /// `yyterminate()` does.
    pub fn next_token(&mut self) -> Token {
        loop {
            let Some(c) = self.peek() else {
                return Token {
                    kind: Kind::End,
                    text: String::new(),
                    literal: Literal::None,
                    line: self.line,
                    col: self.col,
                };
            };

            let start = self.pos;
            let line = self.line;
            let col = self.col;

            // [ \t\v\n\f] { }
            if matches!(c, b' ' | b'\t' | 0x0b | b'\n' | 0x0c) {
                self.bump();
                continue;
            }

            // "//"[^\n]* { }
            if c == b'/' && self.peek_at(1) == Some(b'/') {
                while let Some(c) = self.peek() {
                    if c == b'\n' {
                        break;
                    }
                    self.bump();
                }
                continue;
            }

            // "/*" { comment(); }
            if c == b'/' && self.peek_at(1) == Some(b'*') {
                self.bump();
                self.bump();
                self.skip_block_comment();
                continue;
            }

            // {L}({L}|{D})* -- also covers every keyword, because flex picks the
            // longest match and the keyword rules are all prefixes of it.
            if is_label_start(c) {
                while let Some(c) = self.peek() {
                    if is_label_char(c) {
                        self.bump();
                    }
                    else {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();

                if let Some(&(_, kind)) = KEYWORDS.iter().find(|(kw, _)| *kw == text) {
                    let literal = match kind {
                        Kind::Bool => Literal::Bool(text == "true"),
                        _ => Literal::None,
                    };
                    return Token { kind, text, literal, line, col };
                }

                return Token { kind: Kind::Label, text, literal: Literal::None, line, col };
            }

            if c.is_ascii_digit() {
                // 0[xX]{H}+ -- flex's longest match means the hex rule wins
                // whenever at least one hex digit follows the prefix.
                if c == b'0'
                    && matches!(self.peek_at(1), Some(b'x') | Some(b'X'))
                    && self.peek_at(2).is_some_and(|c| c.is_ascii_hexdigit())
                {
                    self.bump();
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() {
                            self.bump();
                        }
                        else {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                    // stoul(text, nullptr, 16) stored through `unsigned int`
                    // and cast to `int`.
                    let value = u32::from_str_radix(&text[2..], 16).unwrap_or(0) as i32;
                    return Token { kind: Kind::Int, text, literal: Literal::Int(value), line, col };
                }

                let mut end = self.pos;
                while end < self.src.len() && self.src[end].is_ascii_digit() {
                    end += 1;
                }

                // {D}+"."{D}+ wins over {D}+ by longest match.
                if end + 1 < self.src.len()
                    && self.src[end] == b'.'
                    && self.src[end + 1].is_ascii_digit()
                {
                    let mut frac = end + 1;
                    while frac < self.src.len() && self.src[frac].is_ascii_digit() {
                        frac += 1;
                    }
                    while self.pos < frac {
                        self.bump();
                    }
                    let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                    let value = text.parse::<f64>().unwrap_or(0.0);
                    return Token { kind: Kind::Float, text, literal: Literal::Float(value), line, col };
                }

                while self.pos < end {
                    self.bump();
                }
                let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                // stoul(text, nullptr, 10) stored through `unsigned int`, cast to `int`.
                let value = text.parse::<u32>().unwrap_or(0) as i32;
                return Token { kind: Kind::Int, text, literal: Literal::Int(value), line, col };
            }

            // \"(\\.|[^\\\"\n])*\"
            if c == b'"' {
                self.bump();
                loop {
                    match self.peek() {
                        Some(b'"') => {
                            self.bump();
                            let text =
                                String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                            // The scanner keeps the body verbatim: no escape
                            // processing at all.
                            let body = text[1..text.len() - 1].to_string();
                            return Token {
                                kind: Kind::Str,
                                text,
                                literal: Literal::Str(body),
                                line,
                                col,
                            };
                        }
                        Some(b'\\') => {
                            match self.peek_at(1) {
                                // `\\.` in flex: the escaped character may not
                                // be a newline.
                                Some(b'\n') | None => break,
                                Some(_) => {
                                    self.bump();
                                    self.bump();
                                }
                            }
                        }
                        Some(b'\n') | None => break,
                        Some(_) => {
                            self.bump();
                        }
                    }
                }

                // No closing quote: the `"` itself falls through to the
                // catch-all rule, which reports a single unrecognized byte.
                self.pos = start;
                self.line = line;
                self.col = col;
                self.bump();
                return self.token(Kind::Unrecognized, line, col, start);
            }

            // Two byte operators first: flex prefers the longest match, so
            // `=>` must not be lexed as `=` followed by `>`.
            if c == b'=' && self.peek_at(1) == Some(b'>') {
                self.bump();
                self.bump();
                return self.token(Kind::BuiltinPointer, line, col, start);
            }

            if c == b':' && self.peek_at(1) == Some(b':') {
                self.bump();
                self.bump();
                return self.token(Kind::NamespacePointer, line, col, start);
            }

            let (kind, len): (Kind, usize) = match c {
                b'(' => (Kind::LParen, 1),
                b')' => (Kind::RParen, 1),
                b'[' => (Kind::LBracket, 1),
                b']' => (Kind::RBracket, 1),
                b'{' => (Kind::LBrace, 1),
                b'}' => (Kind::RBrace, 1),
                b':' => (Kind::Colon, 1),
                b';' => (Kind::Semi, 1),
                b',' => (Kind::Comma, 1),
                b'^' => (Kind::Caret, 1),
                b'.' => (Kind::Dot, 1),
                b'*' => (Kind::Star, 1),
                b'/' => (Kind::Slash, 1),
                b'+' => (Kind::Plus, 1),
                b'-' => (Kind::Minus, 1),
                b'=' => (Kind::Eq, 1),
                b'@' => (Kind::Decorator, 1),
                // `.` catch-all rule: no rule in scanner.l matches this byte.
                _ => (Kind::Unrecognized, 1),
            };

            for _ in 0..len {
                self.bump();
            }

            return self.token(kind, line, col, start);
        }
    }
}

/// Tokenizes a whole source file.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut scanner = Scanner::new(src);
    let mut tokens = Vec::new();
    loop {
        let token = scanner.next_token();
        let end = token.kind == Kind::End;
        tokens.push(token);
        if end {
            return tokens;
        }
    }
}
