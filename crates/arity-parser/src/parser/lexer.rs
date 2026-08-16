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
    // Roxygen line sub-tokens (see `crate::parser::roxygen`).
    RoxygenMarker,
    RoxygenAt,
    RoxygenTagName,
    RoxygenTagArg,
    RoxygenText,
    RoxygenCode,
    RoxygenRdMacro,
    RoxygenMdLink,
    /// A markdown **inline-link bracket** (`[`/`![` opener, or a `](url)` closer
    /// carrying the destination), recognized only under a resolved `@md` block
    /// mode for the inline `[text](url)` form. The lexer carves the bracket *and*
    /// recursively lexes the link text in between (so emphasis/code inside it
    /// resolve), then the inline pass (`roxygen::inline`) assembles the matched
    /// pair into a `ROXYGEN_MD_LINK` **node** whose display children are the
    /// resolved markdown. A transient kind: every bracket the lexer emits is part
    /// of a complete inline link, so the pass always consumes it (never reaching
    /// the tree builder as a bare token).
    RoxygenMdBracket,
    /// A markdown image `![alt](url "title")`, recognized only under a resolved
    /// `@md` block mode. Projected to `\figure` (extension-keyed `\if` wrapping).
    RoxygenMdImage,
    /// A raw markdown emphasis-delimiter run (`*`, `**`, `_`, `___`, …), emitted
    /// only under a resolved `@md` block mode. The lexer emits the maximal same-
    /// char run *neutrally* — no open/close decision — and the paragraph-level
    /// inline pass (`roxygen::inline`) resolves runs into `ROXYGEN_MD_EMPH`/
    /// `ROXYGEN_MD_STRONG` *nodes* via the CommonMark delimiter-stack algorithm,
    /// leaving unmatched runs as literal `ROXYGEN_MD_DELIM` leaves.
    RoxygenMdDelim,
    /// A markdown code span (`` `x` ``), emitted only under a resolved `@md` block
    /// mode. Projected to `\code`/`\verb` per roxygen2's R-parseability rule.
    RoxygenMdCode,
    /// A markdown list item's leading marker (`-`/`*`/`+` or `1.`/`1)`), at a
    /// line's content start under a resolved `@md` mode. Excludes the trailing
    /// space (kept in the following text run) so a marker that does not form a
    /// list reflows like the plain text it stands in for.
    RoxygenMdListMarker,
    /// A markdown fenced-code-block delimiter line (a run of 3+ backticks, plus
    /// an optional info string), at a line's content start under a resolved
    /// `@md` mode. The whole remaining line content is the token (the opener
    /// carries the info string, the closer is backticks only). The block builder
    /// pairs an opener with its closer into a `ROXYGEN_MD_CODE_BLOCK`.
    RoxygenMdFence,
    /// A raw inline-HTML tag (`<img …>`, `</span>`), recognized only under a
    /// resolved `@md` block mode. Projected to `\if{html}{\out{<tag>}}`.
    RoxygenMdHtml,
    /// A markdown **HTML-block** opener line: a line whose content starts with a
    /// CommonMark HTML-block start (condition 6 — a block-level tag), recognized
    /// only at a line's content start under a resolved `@md` mode. The whole
    /// remaining line content is the token; the block builder gathers the opener
    /// and the following lines (to the next blank line) into a
    /// `ROXYGEN_MD_HTML_BLOCK`.
    RoxygenMdHtmlBlock,
    /// A GFM-table **delimiter row** (`|---|:--:|`): a line whose whole content is
    /// a run of `|`-separated alignment cells (each `:?-+:?`, at least one `|`),
    /// recognized only at a line's content start under a resolved `@md` mode. The
    /// whole remaining line content is the token. The block builder pairs it with
    /// the preceding header line (when their cell counts match) into a
    /// `ROXYGEN_MD_TABLE`; an unmatched delimiter row stays literal prose (the tree
    /// builder maps this kind to `ROXYGEN_TEXT`).
    RoxygenMdTableDelim,
    /// An ATX **heading** line (`# Title`, `## Sub`, up to `######`): a line whose
    /// content starts with a run of 1-6 `#` followed by a space/tab or the end of
    /// the line, recognized only at a line's content start under a resolved `@md`
    /// mode. The whole remaining line content is the token. The block builder wraps
    /// it in a single-line `ROXYGEN_MD_HEADING` node; the tree builder maps this
    /// kind to `ROXYGEN_TEXT`, so a heading leaf that is never wrapped (there is no
    /// such path today) stays literal prose.
    RoxygenMdHeading,
    /// A **setext heading underline** line (`===`/`---`): a line whose content is a
    /// non-empty run of `=` (level 1) or two-or-more `-` (level 2), with only
    /// leading/trailing whitespace, recognized at a line's content start under a
    /// resolved `@md` mode. The token is the whole line content. Whether it actually
    /// forms a heading is a *block-level look-back* decision — a setext underline
    /// heads nothing on its own; it promotes the **preceding paragraph** into a
    /// heading (`emit_md_setext_heading`). The tree builder maps this kind to
    /// `ROXYGEN_TEXT`, so an underline with no preceding paragraph (a thematic-break
    /// position) stays literal prose. Single `-`/`- ` underlines are *not* carved
    /// here — they collide with an empty list-item marker — so a single-dash setext
    /// H2 is deferred backlog; `---` (the common form) is covered.
    RoxygenMdSetextUnderline,
    /// A markdown **block-quote** line (`> quoted`): a line whose content, after up
    /// to three spaces of indentation, begins with `>`, recognized only at a line's
    /// content start under a resolved `@md` mode. The whole remaining line content is
    /// the token. The block builder gathers consecutive block-quote lines into a
    /// `ROXYGEN_MD_BLOCK_QUOTE` node. roxygen2 does not support block quotes: it warns
    /// and renders the node's *flattened plain text* (`escape_comment(xml_text)` — the
    /// `>` markers and inner markup dropped), which the projector reproduces. The tree
    /// builder maps this kind to `ROXYGEN_TEXT`, so a leaf that is somehow never
    /// gathered stays literal prose.
    RoxygenMdBlockQuote,
    /// A markdown **thematic break** line (`***`/`---`/`___`): a line whose whole
    /// content, after up to three spaces of indentation, is three or more of a single
    /// `*`/`-`/`_` character with only spaces/tabs between and after, recognized only
    /// at a line's content start under a resolved `@md` mode. A contiguous `---`/`===`
    /// run is left to `RoxygenMdSetextUnderline` (setext takes precedence), so this
    /// kind carves the `*`/`_`-based and space-separated forms; a bare `---` becomes a
    /// thematic break at block level when it heads no paragraph. The block builder
    /// wraps the line in a single-line `ROXYGEN_MD_THEMATIC_BREAK` node. roxygen2 has
    /// no thematic-break support: it warns and renders empty, so the projector drops
    /// it. The tree builder maps this kind to `ROXYGEN_TEXT`, so a leaf that is somehow
    /// never gathered stays literal prose.
    RoxygenMdThematicBreak,
}

/// The semantic role of a roxygen line sub-token. This is the **single source**
/// for classifying the roxygen `TokKind`s: every site that used to carry its own
/// hand-maintained `matches!` list now derives from [`TokKind::roxygen_role`].
/// `Content` is the prose/inline-markup body (text + protected spans); the
/// others are the structural line tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoxygenRole {
    /// The leading `#'`.
    Marker,
    /// The `@` introducing a tag.
    At,
    /// A tag's name (`@param`'s `param`).
    TagName,
    /// A tag's first-word argument (`@param`'s `x`).
    TagArg,
    /// Prose / inline-markup body: text, inline code, an Rd macro, a markdown
    /// link, or a resolved markdown emphasis/strong/code/list-marker span.
    Content,
}

impl TokKind {
    /// The roxygen role of this kind, or `None` for any non-roxygen token. This
    /// is a wildcard-free match, so adding a `TokKind` is a compile error here —
    /// the one place that must classify a new roxygen kind's role.
    pub(crate) fn roxygen_role(&self) -> Option<RoxygenRole> {
        use TokKind::*;
        match self {
            RoxygenMarker => Some(RoxygenRole::Marker),
            RoxygenAt => Some(RoxygenRole::At),
            RoxygenTagName => Some(RoxygenRole::TagName),
            RoxygenTagArg => Some(RoxygenRole::TagArg),
            RoxygenText
            | RoxygenCode
            | RoxygenRdMacro
            | RoxygenMdLink
            | RoxygenMdBracket
            | RoxygenMdImage
            | RoxygenMdDelim
            | RoxygenMdCode
            | RoxygenMdListMarker
            | RoxygenMdFence
            | RoxygenMdHtml
            | RoxygenMdHtmlBlock
            | RoxygenMdTableDelim
            | RoxygenMdHeading
            | RoxygenMdSetextUnderline
            | RoxygenMdBlockQuote
            | RoxygenMdThematicBreak => Some(RoxygenRole::Content),
            Ident | Int | Float | Complex | String | Comment | IfKw | ElseKw | ForKw | WhileKw
            | RepeatKw | FunctionKw | LambdaFn | InKw | Tilde | Question | UserOp | LBrack
            | RBrack | LBrack2 | RBrack2 | Plus | Minus | Star | Slash | Caret | Pipe | Colon
            | Colon2 | Colon3 | Dollar | At | Semicolon | Comma | Or | Or2 | And | And2
            | Equal2 | NotEqual | Bang | LessThan | LessThanOrEqual | GreaterThan
            | GreaterThanOrEqual | LParen | RParen | LBrace | RBrace | AssignLeft | SuperAssign
            | AssignRight | SuperAssignRight | AssignEq | Walrus | Whitespace | Newline
            | Unknown => None,
        }
    }

    /// Comment-like trivia: a plain comment, or any sub-token of a roxygen
    /// line. Used by trivia-skip loops so a roxygen line appearing where a
    /// comment could (mid-expression) is skipped like one rather than tripping
    /// the parser.
    pub(crate) fn is_comment_like(&self) -> bool {
        matches!(self, TokKind::Comment) || self.roxygen_role().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token<'a> {
    pub(crate) kind: TokKind,
    /// The token's text: a slice of the lexed input, or a `'static` literal
    /// for fixed-text tokens. Redundant with `start..end` but kept so
    /// consumers never re-slice the input; the tree builder copies it into
    /// the green tree, which is where the borrow ends.
    pub(crate) text: &'a str,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// True when the two bytes at `i` are exactly `pat` --- a char-boundary-safe
/// two-char operator lookahead. Slicing `&input[i..i + 2]` panics when `i` (or
/// `i + 1`) lands inside a multibyte UTF-8 char (e.g. a U+00A0 non-breaking
/// space); comparing raw bytes never does, and every operator we scan for is
/// two ASCII bytes.
#[inline]
fn two_bytes(bytes: &[u8], i: usize, pat: &[u8; 2]) -> bool {
    bytes.get(i) == Some(&pat[0]) && bytes.get(i + 1) == Some(&pat[1])
}

/// [`two_bytes`] for a three-ASCII-byte operator (`:::`, `<<-`, `->>`).
#[inline]
fn three_bytes(bytes: &[u8], i: usize, pat: &[u8; 3]) -> bool {
    bytes.get(i) == Some(&pat[0])
        && bytes.get(i + 1) == Some(&pat[1])
        && bytes.get(i + 2) == Some(&pat[2])
}

#[cfg(test)]
pub(crate) fn lex(input: &str) -> Vec<Token<'_>> {
    lex_with_md(input, false)
}

/// Lex `input` with `md_default` as the markdown mode of roxygen blocks
/// carrying no `@md`/`@noMd` directive (see
/// [`ParseOptions`](crate::parser::core::ParseOptions)).
pub(crate) fn lex_with_md(input: &str, md_default: bool) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let bytes = input.as_bytes();
    // The resolved markdown mode of the roxygen block currently being lexed, and
    // the byte offset past that block. Resolved once per block (see
    // `resolve_roxygen_block`) and reused for every line until `i` reaches the
    // block end, so a `@md` directive anywhere in a block keys the whole block.
    let mut rox_md = false;
    let mut rox_block_end = 0usize;
    // Whether the current line belongs to a tag whose body is never markdown even
    // when the block is `@md` — verbatim Rd (`@rawRd`) or verbatim code
    // (`@examples`, …). Reset at each block boundary; a line opening a tag resets
    // it to that tag's setting, while a prose/continuation line keeps the
    // enclosing tag's.
    let mut rox_no_md = false;

    while i < bytes.len() {
        let c = bytes[i] as char;

        match c {
            '\r' => {
                if i + 1 < bytes.len() && (bytes[i + 1] as char) == '\n' {
                    out.push(Token {
                        kind: TokKind::Newline,
                        text: "\r\n",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    out.push(Token {
                        kind: TokKind::Newline,
                        text: "\r",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '\n' => {
                out.push(Token {
                    kind: TokKind::Newline,
                    text: "\n",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '#' => {
                let start = i;
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j] as char) != '\n' {
                    j += 1;
                }
                // `j` is at the line's `\n` or EOF; the full comment text is
                // `input[start..j]` (it may end with a `\r` under CRLF).
                let line = &input[start..j];
                if crate::parser::roxygen::is_roxygen_comment(line) {
                    // Resolve the markdown mode once per block: when this line
                    // starts a new block (`i` is past the cached block's end),
                    // scan the whole block for an `@md`/`@noMd` directive.
                    if start >= rox_block_end {
                        (rox_md, rox_block_end) =
                            crate::parser::roxygen::resolve_roxygen_block(input, start, md_default);
                        rox_no_md = false;
                    }
                    // Leave a trailing `\r` (and the `\n`) to the main loop so
                    // CRLF stays one Newline token and roxygen content is clean.
                    let content_end = if line.ends_with('\r') { j - 1 } else { j };
                    let line_text = &input[start..content_end];
                    // A line opening a tag re-keys the no-markdown region to that
                    // tag's body (`@rawRd` is verbatim Rd, `@examples` and the
                    // other code tags are verbatim R — neither is markdown even
                    // under `@md`); a prose or continuation line keeps the
                    // enclosing tag's setting.
                    if let Some(tag) = crate::parser::roxygen::roxygen_line_tag(line_text) {
                        rox_no_md = crate::parser::roxygen::tag_body_skips_markdown(tag);
                    }
                    crate::parser::roxygen::lex_roxygen_line(
                        &mut out,
                        line_text,
                        start,
                        rox_md && !rox_no_md,
                    );
                    i = content_end;
                } else {
                    out.push(Token {
                        kind: TokKind::Comment,
                        text: line,
                        start,
                        end: j,
                    });
                    i = j;
                }
            }
            '~' => {
                out.push(Token {
                    kind: TokKind::Tilde,
                    text: "~",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '?' => {
                out.push(Token {
                    kind: TokKind::Question,
                    text: "?",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '$' => {
                out.push(Token {
                    kind: TokKind::Dollar,
                    text: "$",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '@' => {
                out.push(Token {
                    kind: TokKind::At,
                    text: "@",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ';' => {
                out.push(Token {
                    kind: TokKind::Semicolon,
                    text: ";",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ',' => {
                out.push(Token {
                    kind: TokKind::Comma,
                    text: ",",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '+' => {
                out.push(Token {
                    kind: TokKind::Plus,
                    text: "+",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '*' => {
                if i + 1 < bytes.len() && (bytes[i + 1] as char) == '*' {
                    out.push(Token {
                        kind: TokKind::Caret,
                        text: "**",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    out.push(Token {
                        kind: TokKind::Star,
                        text: "*",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '^' => {
                out.push(Token {
                    kind: TokKind::Caret,
                    text: "^",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '(' => {
                out.push(Token {
                    kind: TokKind::LParen,
                    text: "(",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '\\' => {
                out.push(Token {
                    kind: TokKind::LambdaFn,
                    text: "\\",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            ')' => {
                out.push(Token {
                    kind: TokKind::RParen,
                    text: ")",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '{' => {
                out.push(Token {
                    kind: TokKind::LBrace,
                    text: "{",
                    start: i,
                    end: i + 1,
                });
                i += 1;
            }
            '}' => {
                out.push(Token {
                    kind: TokKind::RBrace,
                    text: "}",
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
                        text: &input[start..i],
                        start,
                        end: i,
                    });
                } else {
                    out.push(Token {
                        kind: TokKind::Unknown,
                        text: &input[start..i],
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
                    text: &input[start..i],
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
                    text: &input[start..i],
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
                        text: &input[start..i],
                        start,
                        end: i,
                    });
                    continue;
                }

                if two_bytes(bytes, i, b"||") {
                    out.push(Token {
                        kind: TokKind::Or2,
                        text: "||",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b"&&") {
                    out.push(Token {
                        kind: TokKind::And2,
                        text: "&&",
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
                                text: &input[start..i],
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
                            text: &input[start..i],
                            start,
                            end: i,
                        });
                        continue;
                    }

                    if i + 2 < bytes.len()
                        && (bytes[i + 1] as char) == '.'
                        && (bytes[i + 2] as char).is_ascii_digit()
                    {
                        // `..1` is the dot-dot-i special, but as with `...` above
                        // it can equally be the start of a longer symbol
                        // (`..2dge` in Matrix). Consume the digit run and then any
                        // trailing name characters so the whole symbol is one
                        // identifier; with none, the text is just `..<digits>`.
                        let start = i;
                        i += 3;
                        while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                            i += 1;
                        }
                        while i < bytes.len() {
                            let ch = bytes[i] as char;
                            if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
                                break;
                            }
                            i += 1;
                        }
                        out.push(Token {
                            kind: TokKind::Ident,
                            text: &input[start..i],
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
                            text: &input[start..i],
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
                        text: &input[start..i],
                        start,
                        end: i,
                    });
                    continue;
                }

                if two_bytes(bytes, i, b"==") {
                    out.push(Token {
                        kind: TokKind::Equal2,
                        text: "==",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b"!=") {
                    out.push(Token {
                        kind: TokKind::NotEqual,
                        text: "!=",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '!' {
                    out.push(Token {
                        kind: TokKind::Bang,
                        text: "!",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if three_bytes(bytes, i, b":::") {
                    out.push(Token {
                        kind: TokKind::Colon3,
                        text: ":::",
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if two_bytes(bytes, i, b"::") {
                    out.push(Token {
                        kind: TokKind::Colon2,
                        text: "::",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b":=") {
                    out.push(Token {
                        kind: TokKind::Walrus,
                        text: ":=",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b"|>") {
                    out.push(Token {
                        kind: TokKind::Pipe,
                        text: "|>",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if three_bytes(bytes, i, b"<<-") {
                    out.push(Token {
                        kind: TokKind::SuperAssign,
                        text: "<<-",
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if three_bytes(bytes, i, b"->>") {
                    out.push(Token {
                        kind: TokKind::SuperAssignRight,
                        text: "->>",
                        start: i,
                        end: i + 3,
                    });
                    i += 3;
                    continue;
                }

                if two_bytes(bytes, i, b"<-") {
                    out.push(Token {
                        kind: TokKind::AssignLeft,
                        text: "<-",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b"->") {
                    out.push(Token {
                        kind: TokKind::AssignRight,
                        text: "->",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '-' {
                    out.push(Token {
                        kind: TokKind::Minus,
                        text: "-",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '/' {
                    out.push(Token {
                        kind: TokKind::Slash,
                        text: "/",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == ':' {
                    out.push(Token {
                        kind: TokKind::Colon,
                        text: ":",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if two_bytes(bytes, i, b"<=") {
                    out.push(Token {
                        kind: TokKind::LessThanOrEqual,
                        text: "<=",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b">=") {
                    out.push(Token {
                        kind: TokKind::GreaterThanOrEqual,
                        text: ">=",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '=' {
                    out.push(Token {
                        kind: TokKind::AssignEq,
                        text: "=",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '|' {
                    out.push(Token {
                        kind: TokKind::Or,
                        text: "|",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '&' {
                    out.push(Token {
                        kind: TokKind::And,
                        text: "&",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '<' {
                    out.push(Token {
                        kind: TokKind::LessThan,
                        text: "<",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == '>' {
                    out.push(Token {
                        kind: TokKind::GreaterThan,
                        text: ">",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if two_bytes(bytes, i, b"[[") {
                    out.push(Token {
                        kind: TokKind::LBrack2,
                        text: "[[",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if two_bytes(bytes, i, b"]]") {
                    out.push(Token {
                        kind: TokKind::RBrack2,
                        text: "]]",
                        start: i,
                        end: i + 2,
                    });
                    i += 2;
                    continue;
                }

                if c == '[' {
                    out.push(Token {
                        kind: TokKind::LBrack,
                        text: "[",
                        start: i,
                        end: i + 1,
                    });
                    i += 1;
                    continue;
                }

                if c == ']' {
                    out.push(Token {
                        kind: TokKind::RBrack,
                        text: "]",
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
                            text: &input[start..i],
                            start,
                            end: i,
                        });
                    } else {
                        while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                            i += 1;
                        }

                        // R's decimal grammar is `[0-9]+ '.' [0-9]*` — the
                        // fractional digits are optional, so a trailing dot is
                        // part of the literal (`1.` is a double, `1.e3` is
                        // scientific), not a following `.` symbol.
                        let mut is_float = false;
                        if i < bytes.len() && (bytes[i] as char) == '.' {
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
                            text: &input[start..i],
                            start,
                            end: i,
                        });
                    }
                    continue;
                }

                // R raw strings: `[rR]"[-]*(content)[-]*"` where the opening
                // bracket is one of `(`, `[`, `{` (closed by the matching
                // `)`, `]`, `}`), the quote is `"` or `'`, and the run of dashes
                // on the closing side matches the opening run. E.g. `r"(x)"`,
                // `R"[x]"`, `r"---{x}---"`, `r'(x)'`.
                if (c == 'r' || c == 'R')
                    && i + 1 < bytes.len()
                    && matches!(bytes[i + 1] as char, '"' | '\'')
                {
                    let start = i;
                    let quote = bytes[i + 1] as char;
                    let mut j = i + 2;
                    // Opening dash run.
                    let dash_start = j;
                    while j < bytes.len() && (bytes[j] as char) == '-' {
                        j += 1;
                    }
                    let dash_len = j - dash_start;
                    let mut matched_raw = false;

                    let close = match bytes.get(j).map(|b| *b as char) {
                        Some('(') => Some(')'),
                        Some('[') => Some(']'),
                        Some('{') => Some('}'),
                        _ => None,
                    };

                    if let Some(close_ch) = close {
                        let mut k = j + 1;
                        while k < bytes.len() {
                            if (bytes[k] as char) == close_ch {
                                let after_close = k + 1;
                                let dash_end = after_close + dash_len;
                                if dash_end < bytes.len()
                                    && bytes[after_close..dash_end].iter().all(|b| *b == b'-')
                                    && (bytes[dash_end] as char) == quote
                                {
                                    let end = dash_end + 1;
                                    out.push(Token {
                                        kind: TokKind::String,
                                        text: &input[start..end],
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
                        text,
                        start,
                        end: i,
                    });
                    continue;
                }

                // Catch-all. `c` (= `bytes[i] as char`) is only a faithful char
                // for ASCII; a non-ASCII byte begins a multibyte UTF-8 char, so
                // read the whole char from the source and advance by its full
                // width. Emitting one byte at a time would Latin-1-mangle it (a
                // U+00A0 → `Â` + U+00A0) and leave `i` mid-char for the next
                // iteration --- both a losslessness break. `i` is always on a
                // char boundary here, so `chars().next()` cannot fail.
                let ch = input[i..].chars().next().unwrap_or(c);
                let len = ch.len_utf8();
                out.push(Token {
                    kind: TokKind::Unknown,
                    text: &input[i..i + len],
                    start: i,
                    end: i + len,
                });
                i += len;
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

    /// R raw strings allow the `R` prefix, `'` quotes, and `[`/`{` bracket
    /// delimiters (each with an optional matching dash run), not just `r"(...)"`.
    #[test]
    fn lexes_all_raw_string_delimiter_forms() {
        for src in [
            "r\"{a}\"",
            "r\"[a]\"",
            "R\"(a)\"",
            "r'(a)'",
            "r\"---{a}---\"",
            "r\"--[a]--\"",
            "r\"{.*\\s*X\\s*}\"",
        ] {
            let tokens = lex(src);
            assert_eq!(tokens.len(), 1, "{src:?} should lex as one token");
            assert_eq!(tokens[0].kind, TokKind::String, "{src:?} kind");
            assert_eq!(tokens[0].text, src, "{src:?} text");
        }
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

    /// `..2` is the dot-dot-i special only when the *whole* name is dots plus
    /// digits. `..2dge` is an ordinary R symbol (Matrix has one), so the digit
    /// run must not cut the identifier short.
    #[test]
    fn lexes_dots_digits_symbols_with_trailing_name_chars_as_one_ident() {
        for (input, expected) in [
            ("..2dge", "..2dge"),
            ("..1foo", "..1foo"),
            ("..2.bar", "..2.bar"),
            ("..10_x", "..10_x"),
            ("..2", "..2"),
        ] {
            let tokens = lex(input);
            assert_eq!(tokens.len(), 1, "{input:?} should lex as one token");
            assert_eq!(tokens[0].kind, TokKind::Ident);
            assert_eq!(tokens[0].text, expected);
        }
    }

    /// R's numeric grammar is `[0-9]+ '.' [0-9]*` — the fractional digits are
    /// optional, so a trailing dot belongs to the literal (`1.` is a double,
    /// not `1` followed by the symbol `.`).
    #[test]
    fn lexes_trailing_dot_numerics_as_single_float_tokens() {
        for (input, expected) in [("1.", "1."), ("10.", "10."), ("1.e3", "1.e3")] {
            let tokens = lex(input);
            assert_eq!(tokens.len(), 1, "{input:?} should lex as one token");
            assert_eq!(tokens[0].kind, TokKind::Float, "{input:?}");
            assert_eq!(tokens[0].text, expected);
        }

        // The integer suffix still applies, and `1.` before `;` stops at the `;`.
        let tokens = lex("1.;");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].kind, TokKind::Float);
        assert_eq!(tokens[0].text, "1.");
        assert_eq!(tokens[1].kind, TokKind::Semicolon);
    }

    #[test]
    fn lexes_multibyte_char_before_two_char_operator_without_panicking() {
        // A U+00A0 (2-byte non-breaking space) sitting just before a two-char
        // operator must not make the operator lookahead slice inside the char.
        // Lossless round-trip (every byte accounted for) confirms no panic and no
        // dropped input.
        for input in ["\u{a0}||", "a\u{a0}&&b", "\u{a0}<-1", "x\u{a0}[[1]]"] {
            let reconstructed: String = lex(input).iter().map(|t| t.text).collect();
            assert_eq!(reconstructed, input, "lossless lex of {input:?}");
        }
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
