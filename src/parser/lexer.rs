#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokKind {
    Ident,
    Int,
    Float,
    Complex,
    String,
    Comment,
    IfKw,
    ElseKw,
    ForKw,
    WhileKw,
    RepeatKw,
    FunctionKw,
    LambdaFn,
    InKw,
    Tilde,
    Question,
    UserOp,
    LBrack,
    RBrack,
    LBrack2,
    RBrack2,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Pipe,
    Colon,
    Colon2,
    Colon3,
    Dollar,
    At,
    Semicolon,
    Comma,
    Or,
    Or2,
    And,
    And2,
    Equal2,
    NotEqual,
    Bang,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LParen,
    RParen,
    LBrace,
    RBrace,
    AssignLeft,
    SuperAssign,
    AssignRight,
    SuperAssignRight,
    AssignEq,
    Walrus,
    Whitespace,
    Newline,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: TokKind,
    pub(crate) text: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn lex(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        let c = bytes[i] as char;

        match c {
            '\r' => {
                if i + 1 < bytes.len() && (bytes[i + 1] as char) == '\n' {
                    out.push(Token {
                        kind: TokKind::Newline,
                        text: "\r\n".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    out.push(Token {
                        kind: TokKind::Newline,
                        text: "\r".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '\n' => {
                out.push(Token {
                    kind: TokKind::Newline,
                    text: "\n".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '#' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i] as char) != '\n' {
                    i += 1;
                }
                out.push(Token {
                    kind: TokKind::Comment,
                    text: input[start..i].to_string(),
                    start,
                    end: i,
                });
            }
            '~' => {
                out.push(Token {
                    kind: TokKind::Tilde,
                    text: "~".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '?' => {
                out.push(Token {
                    kind: TokKind::Question,
                    text: "?".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '$' => {
                out.push(Token {
                    kind: TokKind::Dollar,
                    text: "$".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '@' => {
                out.push(Token {
                    kind: TokKind::At,
                    text: "@".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ';' => {
                out.push(Token {
                    kind: TokKind::Semicolon,
                    text: ";".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ',' => {
                out.push(Token {
                    kind: TokKind::Comma,
                    text: ",".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '+' => {
                out.push(Token {
                    kind: TokKind::Plus,
                    text: "+".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '*' => {
                if i + 1 < bytes.len() && (bytes[i + 1] as char) == '*' {
                    out.push(Token {
                        kind: TokKind::Caret,
                        text: "**".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    out.push(Token {
                        kind: TokKind::Star,
                        text: "*".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '^' => {
                out.push(Token {
                    kind: TokKind::Caret,
                    text: "^".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '(' => {
                out.push(Token {
                    kind: TokKind::LParen,
                    text: "(".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '\\' => {
                out.push(Token {
                    kind: TokKind::LambdaFn,
                    text: "\\".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ')' => {
                out.push(Token {
                    kind: TokKind::RParen,
                    text: ")".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '{' => {
                out.push(Token {
                    kind: TokKind::LBrace,
                    text: "{".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '}' => {
                out.push(Token {
                    kind: TokKind::RBrace,
                    text: "}".to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '%' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i] as char) != '%' {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] as char) == '%' {
                    i += 1;
                    out.push(Token {
                        kind: TokKind::UserOp,
                        text: input[start..i].to_string(),
                        start,
                        end: i,
                    });
                } else {
                    out.push(Token {
                        kind: TokKind::Unknown,
                        text: input[start..i].to_string(),
                        start,
                        end: i,
                    });
                }
            }
            '"' | '\'' => {
                let quote = c;
                let start = i;
                i += 1;
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    if ch == '\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if ch == quote {
                        break;
                    }
                }
                out.push(Token {
                    kind: TokKind::String,
                    text: input[start..i].to_string(),
                    start,
                    end: i,
                });
            }
            '`' => {
                // Backtick-quoted (non-syntactic) names are identifiers in every
                // position a bare name can appear, so lex them as `Ident` with
                // the backticks kept in the text. Backslash escapes are honored,
                // mirroring the string lexer; an unterminated name runs to EOF so
                // losslessness still holds.
                let start = i;
                i += 1;
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    if ch == '\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if ch == '`' {
                        break;
                    }
                }
                out.push(Token {
                    kind: TokKind::Ident,
                    text: input[start..i].to_string(),
                    start,
                    end: i,
                });
            }
            _ => {
                if c.is_ascii_whitespace() {
                    let start = i;
                    while i < bytes.len() {
                        let ch = bytes[i] as char;
                        if ch == '\n' || ch == '\r' || !ch.is_ascii_whitespace() {
                            break;
                        }
                        i += 1;
                    }
                    out.push(Token {
                        kind: TokKind::Whitespace,
                        text: input[start..i].to_string(),
                        start,
                        end: i,
                    });
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "||" {
                    out.push(Token {
                        kind: TokKind::Or2,
                        text: "||".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "&&" {
                    out.push(Token {
                        kind: TokKind::And2,
                        text: "&&".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '.' {
                    if i + 1 < bytes.len() {
                        let next = bytes[i + 1] as char;
                        if next.is_ascii_alphabetic() || next == '_' {
                            let start = i;
                            i += 2;
                            while i < bytes.len() {
                                let ch = bytes[i] as char;
                                if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                                    break;
                                }
                                i += 1;
                            }
                            out.push(Token {
                                kind: TokKind::Ident,
                                text: input[start..i].to_string(),
                                start,
                                end: i,
                            });
                            continue;
                        }
                    }

                    if i + 2 < bytes.len()
                        && (bytes[i + 1] as char) == '.'
                        && (bytes[i + 2] as char) == '.'
                    {
                        // `...` is the dots special, but it can also be the start
                        // of a longer name (`...length`, `...elt`, `...names` are
                        // base-R functions). Consume any trailing name characters
                        // so the whole symbol lexes as one identifier; with none,
                        // the text is just `...`.
                        let start = i;
                        i += 3;
                        while i < bytes.len() {
                            let ch = bytes[i] as char;
                            if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                                break;
                            }
                            i += 1;
                        }
                        out.push(Token {
                            kind: TokKind::Ident,
                            text: input[start..i].to_string(),
                            start,
                            end: i,
                        });
                        continue;
                    }

                    if i + 2 < bytes.len()
                        && (bytes[i + 1] as char) == '.'
                        && (bytes[i + 2] as char).is_ascii_digit()
                    {
                        let start = i;
                        i += 3;
                        while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                            i += 1;
                        }
                        out.push(Token {
                            kind: TokKind::Ident,
                            text: input[start..i].to_string(),
                            start,
                            end: i,
                        });
                        continue;
                    }

                    // Any remaining dot run not immediately followed by a digit is
                    // an identifier: `.`, `..`, and `..name` are all valid R names
                    // (the `.()` in bquote, the magrittr `.`, etc.). A dot followed
                    // by a digit (`.5`) is a numeric literal handled below.
                    if !(i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit()) {
                        let start = i;
                        i += 1;
                        while i < bytes.len() {
                            let ch = bytes[i] as char;
                            if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                                break;
                            }
                            i += 1;
                        }
                        out.push(Token {
                            kind: TokKind::Ident,
                            text: input[start..i].to_string(),
                            start,
                            end: i,
                        });
                        continue;
                    }

                    // A dot immediately followed by a digit is a fractional
                    // numeric literal with no leading zero (`.5`, `.001`,
                    // `.5e-3`, `.5i`). R has no dot-leading integer, so this is
                    // always a float (or imaginary). Mirrors the fractional and
                    // exponent handling in the digit-led number branch below.
                    let start = i;
                    i += 1; // consume the '.'
                    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                        i += 1;
                    }

                    if i < bytes.len() && matches!(bytes[i] as char, 'e' | 'E') {
                        let exp_start = i;
                        let mut j = i + 1;
                        if j < bytes.len() && matches!(bytes[j] as char, '+' | '-') {
                            j += 1;
                        }
                        let mut has_exp_digits = false;
                        while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                            has_exp_digits = true;
                            j += 1;
                        }
                        if has_exp_digits {
                            i = j;
                        } else {
                            i = exp_start;
                        }
                    }

                    let is_complex = i < bytes.len() && (bytes[i] as char) == 'i';
                    if is_complex {
                        i += 1;
                    }

                    out.push(Token {
                        kind: if is_complex {
                            TokKind::Complex
                        } else {
                            TokKind::Float
                        },
                        text: input[start..i].to_string(),
                        start,
                        end: i,
                    });
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "==" {
                    out.push(Token {
                        kind: TokKind::Equal2,
                        text: "==".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "!=" {
                    out.push(Token {
                        kind: TokKind::NotEqual,
                        text: "!=".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '!' {
                    out.push(Token {
                        kind: TokKind::Bang,
                        text: "!".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if i + 2 < bytes.len() && &input[i..i + 3] == ":::" {
                    out.push(Token {
                        kind: TokKind::Colon3,
                        text: ":::".to_string(),
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "::" {
                    out.push(Token {
                        kind: TokKind::Colon2,
                        text: "::".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == ":=" {
                    out.push(Token {
                        kind: TokKind::Walrus,
                        text: ":=".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "|>" {
                    out.push(Token {
                        kind: TokKind::Pipe,
                        text: "|>".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 2 < bytes.len() && &input[i..i + 3] == "<<-" {
                    out.push(Token {
                        kind: TokKind::SuperAssign,
                        text: "<<-".to_string(),
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if i + 2 < bytes.len() && &input[i..i + 3] == "->>" {
                    out.push(Token {
                        kind: TokKind::SuperAssignRight,
                        text: "->>".to_string(),
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "<-" {
                    out.push(Token {
                        kind: TokKind::AssignLeft,
                        text: "<-".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "->" {
                    out.push(Token {
                        kind: TokKind::AssignRight,
                        text: "->".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '-' {
                    out.push(Token {
                        kind: TokKind::Minus,
                        text: "-".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '/' {
                    out.push(Token {
                        kind: TokKind::Slash,
                        text: "/".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == ':' {
                    out.push(Token {
                        kind: TokKind::Colon,
                        text: ":".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "<=" {
                    out.push(Token {
                        kind: TokKind::LessThanOrEqual,
                        text: "<=".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == ">=" {
                    out.push(Token {
                        kind: TokKind::GreaterThanOrEqual,
                        text: ">=".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '=' {
                    out.push(Token {
                        kind: TokKind::AssignEq,
                        text: "=".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '|' {
                    out.push(Token {
                        kind: TokKind::Or,
                        text: "|".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '&' {
                    out.push(Token {
                        kind: TokKind::And,
                        text: "&".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '<' {
                    out.push(Token {
                        kind: TokKind::LessThan,
                        text: "<".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '>' {
                    out.push(Token {
                        kind: TokKind::GreaterThan,
                        text: ">".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "[[" {
                    out.push(Token {
                        kind: TokKind::LBrack2,
                        text: "[[".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if i + 1 < bytes.len() && &input[i..i + 2] == "]]" {
                    out.push(Token {
                        kind: TokKind::RBrack2,
                        text: "]]".to_string(),
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '[' {
                    out.push(Token {
                        kind: TokKind::LBrack,
                        text: "[".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == ']' {
                    out.push(Token {
                        kind: TokKind::RBrack,
                        text: "]".to_string(),
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c.is_ascii_digit() {
                    let start = i;
                    i += 1;
                    let mut force_int = false;

                    // Hex numeric literals: 0x... with optional binary exponent p[+/-]...
                    if i < bytes.len()
                        && bytes[start] as char == '0'
                        && matches!(bytes[i] as char, 'x' | 'X')
                    {
                        i += 1; // consume x/X
                        while i < bytes.len() && (bytes[i] as char).is_ascii_hexdigit() {
                            i += 1;
                        }

                        if i < bytes.len() && (bytes[i] as char) == '.' {
                            i += 1;
                            while i < bytes.len() && (bytes[i] as char).is_ascii_hexdigit() {
                                i += 1;
                            }
                        }

                        if i < bytes.len() && matches!(bytes[i] as char, 'p' | 'P') {
                            i += 1;
                            if i < bytes.len() && matches!(bytes[i] as char, '+' | '-') {
                                i += 1;
                            }
                            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                                i += 1;
                            }
                        }

                        let mut is_complex = false;
                        if i < bytes.len() && matches!(bytes[i] as char, 'L' | 'l') {
                            force_int = true;
                            i += 1;
                        } else if i < bytes.len() && (bytes[i] as char) == 'i' {
                            is_complex = true;
                            i += 1;
                        }

                        out.push(Token {
                            kind: if is_complex {
                                TokKind::Complex
                            } else if force_int {
                                TokKind::Int
                            } else {
                                // R hex numeric constants are doubles unless integer-suffixed.
                                TokKind::Float
                            },
                            text: input[start..i].to_string(),
                            start,
                            end: i,
                        });
                    } else {
                        while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                            i += 1;
                        }

                        let mut is_float = false;
                        if i < bytes.len()
                            && (bytes[i] as char) == '.'
                            && i + 1 < bytes.len()
                            && (bytes[i + 1] as char).is_ascii_digit()
                        {
                            is_float = true;
                            i += 1;
                            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                                i += 1;
                            }
                        }

                        if i < bytes.len() && matches!(bytes[i] as char, 'e' | 'E') {
                            let exp_start = i;
                            let mut j = i + 1;
                            if j < bytes.len() && matches!(bytes[j] as char, '+' | '-') {
                                j += 1;
                            }
                            let mut has_exp_digits = false;
                            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                                has_exp_digits = true;
                                j += 1;
                            }
                            if has_exp_digits {
                                is_float = true;
                                i = j;
                            } else {
                                i = exp_start;
                            }
                        }

                        let mut is_complex = false;
                        if i < bytes.len() && matches!(bytes[i] as char, 'L' | 'l') {
                            force_int = true;
                            i += 1;
                        } else if i < bytes.len() && (bytes[i] as char) == 'i' {
                            is_complex = true;
                            i += 1;
                        }

                        out.push(Token {
                            kind: if is_complex {
                                TokKind::Complex
                            } else if force_int {
                                TokKind::Int
                            } else if is_float {
                                TokKind::Float
                            } else {
                                TokKind::Int
                            },
                            text: input[start..i].to_string(),
                            start,
                            end: i,
                        });
                    }
                    continue;
                }

                // R raw strings: r"delimiter(content)delimiter"
                if c == 'r' && i + 1 < bytes.len() && (bytes[i + 1] as char) == '"' {
                    let start = i;
                    let mut j = i + 2;
                    let delim_start = j;
                    let mut matched_raw = false;
                    while j < bytes.len() && (bytes[j] as char) != '(' {
                        let ch = bytes[j] as char;
                        if ch == '"' || ch == '\n' || ch == '\r' {
                            break;
                        }
                        j += 1;
                    }

                    if j < bytes.len() && (bytes[j] as char) == '(' {
                        let delimiter = &input[delim_start..j];
                        let mut k = j + 1;
                        while k < bytes.len() {
                            if (bytes[k] as char) == ')' {
                                let after_close = k + 1;
                                let delim_end = after_close + delimiter.len();
                                if delim_end < bytes.len()
                                    && &input[after_close..delim_end] == delimiter
                                    && (bytes[delim_end] as char) == '"'
                                {
                                    let end = delim_end + 1;
                                    out.push(Token {
                                        kind: TokKind::String,
                                        text: input[start..end].to_string(),
                                        start,
                                        end,
                                    });
                                    i = end;
                                    matched_raw = true;
                                    break;
                                }
                            }
                            k += 1;
                        }
                    }

                    if matched_raw {
                        continue;
                    }
                }

                if c.is_ascii_alphabetic() || c == '_' {
                    let start = i;
                    i += 1;
                    while i < bytes.len() {
                        let ch = bytes[i] as char;
                        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                            break;
                        }
                        i += 1;
                    }
                    let text = &input[start..i];
                    let kind = match text {
                        "if" => TokKind::IfKw,
                        "else" => TokKind::ElseKw,
                        "for" => TokKind::ForKw,
                        "while" => TokKind::WhileKw,
                        "repeat" => TokKind::RepeatKw,
                        "function" => TokKind::FunctionKw,
                        "in" => TokKind::InKw,
                        _ => TokKind::Ident,
                    };
                    out.push(Token {
                        kind,
                        text: text.to_string(),
                        start,
                        end: i,
                    });
                    continue;
                }

                out.push(Token {
                    kind: TokKind::Unknown,
                    text: c.to_string(),
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{TokKind, lex};

    #[test]
    fn lexes_crlf_as_single_newline_token() {
        let tokens = lex("x\r\ny");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokKind::Ident);
        assert_eq!(tokens[1].kind, TokKind::Newline);
        assert_eq!(tokens[1].text, "\r\n");
        assert_eq!(tokens[2].kind, TokKind::Ident);
    }

    #[test]
    fn lexes_lone_cr_as_newline_token() {
        let tokens = lex("x\ry");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokKind::Ident);
        assert_eq!(tokens[1].kind, TokKind::Newline);
        assert_eq!(tokens[1].text, "\r");
        assert_eq!(tokens[2].kind, TokKind::Ident);
    }

    #[test]
    fn lexes_dotted_identifier_as_single_ident_token() {
        let tokens = lex("is.null(x)");
        assert_eq!(tokens[0].kind, TokKind::Ident);
        assert_eq!(tokens[0].text, "is.null");
    }

    #[test]
    fn lexes_scientific_and_hex_doubles_as_single_float_tokens() {
        let tokens = lex("1e6 0x123F 0x0p+123");
        let number_tokens: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();

        assert_eq!(number_tokens.len(), 3);
        assert_eq!(number_tokens[0].kind, TokKind::Float);
        assert_eq!(number_tokens[0].text, "1e6");
        assert_eq!(number_tokens[1].kind, TokKind::Float);
        assert_eq!(number_tokens[1].text, "0x123F");
        assert_eq!(number_tokens[2].kind, TokKind::Float);
        assert_eq!(number_tokens[2].text, "0x0p+123");
    }

    #[test]
    fn lexes_integer_suffix_literals_as_single_int_tokens() {
        let tokens = lex("1L 1e5L 0x123L 0x0p+10L");
        let number_tokens: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();

        assert_eq!(number_tokens.len(), 4);
        for tok in &number_tokens {
            assert_eq!(tok.kind, TokKind::Int);
        }
        assert_eq!(number_tokens[0].text, "1L");
        assert_eq!(number_tokens[1].text, "1e5L");
        assert_eq!(number_tokens[2].text, "0x123L");
        assert_eq!(number_tokens[3].text, "0x0p+10L");
    }

    #[test]
    fn lexes_imaginary_suffix_literals_as_single_complex_tokens() {
        let tokens = lex("1i 2.5i 1e6i 0x123Fi");
        let number_tokens: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();

        assert_eq!(number_tokens.len(), 4);
        for tok in &number_tokens {
            assert_eq!(tok.kind, TokKind::Complex);
        }
        assert_eq!(number_tokens[0].text, "1i");
        assert_eq!(number_tokens[1].text, "2.5i");
        assert_eq!(number_tokens[2].text, "1e6i");
        assert_eq!(number_tokens[3].text, "0x123Fi");
    }

    #[test]
    fn lexes_raw_strings_as_single_string_tokens() {
        let tokens = lex("r\"(hi)\" r\"-(a)-\" r\"(multi\nline)\"");
        let string_tokens: Vec<_> = tokens
            .into_iter()
            .filter(|t| matches!(t.kind, TokKind::String))
            .collect();
        assert_eq!(string_tokens.len(), 3);
        assert_eq!(string_tokens[0].text, "r\"(hi)\"");
        assert_eq!(string_tokens[1].text, "r\"-(a)-\"");
        assert_eq!(string_tokens[2].text, "r\"(multi\nline)\"");
    }

    #[test]
    fn lexes_dots_symbols_as_ident_tokens() {
        let tokens = lex("... ..1 ..123");
        let sig: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();
        assert_eq!(sig.len(), 3);
        assert_eq!(sig[0].kind, TokKind::Ident);
        assert_eq!(sig[0].text, "...");
        assert_eq!(sig[1].kind, TokKind::Ident);
        assert_eq!(sig[1].text, "..1");
        assert_eq!(sig[2].kind, TokKind::Ident);
        assert_eq!(sig[2].text, "..123");
    }

    #[test]
    fn lexes_double_star_as_single_caret_token() {
        let tokens = lex("1**2 1 * *2");
        let sig: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();
        assert_eq!(sig[0].kind, TokKind::Int);
        assert_eq!(sig[1].kind, TokKind::Caret);
        assert_eq!(sig[1].text, "**");
        assert_eq!(sig[2].kind, TokKind::Int);
        assert_eq!(sig[3].kind, TokKind::Int);
        assert_eq!(sig[4].kind, TokKind::Star);
        assert_eq!(sig[4].text, "*");
        assert_eq!(sig[5].kind, TokKind::Star);
        assert_eq!(sig[5].text, "*");
        assert_eq!(sig[6].kind, TokKind::Int);
    }

    #[test]
    fn lexes_backtick_quoted_names_as_ident_tokens() {
        let tokens = lex("`a b` <- `_x`(`if`)");
        let sig: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();
        assert_eq!(sig[0].kind, TokKind::Ident);
        assert_eq!(sig[0].text, "`a b`");
        assert_eq!(sig[1].kind, TokKind::AssignLeft);
        assert_eq!(sig[2].kind, TokKind::Ident);
        assert_eq!(sig[2].text, "`_x`");
        // A backtick name spelled like a keyword stays an identifier.
        assert_eq!(sig[4].kind, TokKind::Ident);
        assert_eq!(sig[4].text, "`if`");
    }

    #[test]
    fn lexes_unterminated_backtick_name_to_end_of_input() {
        let tokens = lex("`oops");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokKind::Ident);
        assert_eq!(tokens[0].text, "`oops");
    }

    #[test]
    fn lexes_lambda_fn_and_dot_prefixed_ident() {
        let tokens = lex("\\(x) .f");
        let sig: Vec<_> = tokens
            .into_iter()
            .filter(|t| !matches!(t.kind, TokKind::Whitespace))
            .collect();
        assert_eq!(sig.len(), 5);
        assert_eq!(sig[0].kind, TokKind::LambdaFn);
        assert_eq!(sig[0].text, "\\");
        assert_eq!(sig[4].kind, TokKind::Ident);
        assert_eq!(sig[4].text, ".f");
    }
}
