use rowan::Language;

pub mod ptr;

pub use ptr::NodePtr;

#[allow(non_camel_case_types)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(u16)]
pub enum SyntaxKind {
    ROOT,
    BINARY_EXPR,
    ASSIGNMENT_EXPR,
    PAREN_EXPR,
    IF_EXPR,
    FOR_EXPR,
    WHILE_EXPR,
    REPEAT_EXPR,
    FUNCTION_EXPR,
    BLOCK_EXPR,
    UNARY_EXPR,
    IDENT,
    INT,
    FLOAT,
    STRING,
    COMMENT,
    TILDE,
    USER_OP,
    LBRACK,
    RBRACK,
    LBRACK2,
    RBRACK2,
    PLUS,
    MINUS,
    STAR,
    SLASH,
    CARET,
    PIPE,
    COLON,
    COLON2,
    COLON3,
    DOLLAR,
    AT,
    SEMICOLON,
    COMMA,
    OR,
    OR2,
    AND,
    AND2,
    EQUAL2,
    NOT_EQUAL,
    BANG,
    LESS_THAN,
    LESS_THAN_OR_EQUAL,
    GREATER_THAN,
    GREATER_THAN_OR_EQUAL,
    LPAREN,
    RPAREN,
    IF_KW,
    ELSE_KW,
    FOR_KW,
    WHILE_KW,
    REPEAT_KW,
    FUNCTION_KW,
    IN_KW,
    LBRACE,
    RBRACE,
    WHITESPACE,
    NEWLINE,
    ASSIGN_LEFT,
    SUPER_ASSIGN,
    ASSIGN_RIGHT,
    SUPER_ASSIGN_RIGHT,
    ASSIGN_EQ,
    CALL_EXPR,
    SUBSET_EXPR,
    SUBSET2_EXPR,
    ARG_LIST,
    ARG,
    ERROR,
    COMPLEX,
    QUESTION,
    WALRUS,
    // Roxygen tokens (leaves). A roxygen line (`#'` …) is sub-tokenized so its
    // structure lives in the CST; the texts of these tokens tile the line's
    // bytes exactly (losslessness).
    ROXYGEN_MARKER,
    ROXYGEN_AT,
    ROXYGEN_TAG_NAME,
    ROXYGEN_TAG_ARG,
    ROXYGEN_TEXT,
    // Roxygen protected-span leaves: inline markup carved out of a `ROXYGEN_TEXT`
    // run so the formatter can treat each as an atomic unit during prose reflow
    // (and a future linter can resolve the code references inside them). Each
    // holds its whole span, delimiters included, so the run still tiles exactly.
    ROXYGEN_CODE,
    ROXYGEN_RD_MACRO,
    ROXYGEN_MD_LINK,
    // Roxygen nodes.
    ROXYGEN_BLOCK,
    /// Reserved: the legacy physical-line node. No longer emitted since the
    /// CST re-model (a roxygen block now owns logical content --- sections and
    /// paragraphs --- with `#'` markers and newlines threaded in as trivia, the
    /// way rowan/rust-analyzer trees attach whitespace). Kept in the enum so the
    /// `as u16` discriminants of the later variants stay stable.
    ROXYGEN_LINE,
    ROXYGEN_TAG,
    // Inline Rd-macro structure. A `ROXYGEN_RD_MACRO` is materialized as a *node*
    // (not a leaf) whose children carve the macro into its parts so the CST models
    // what `tools::parse_Rd` parses: `\code{\link{x}}` becomes nested macro nodes,
    // not flat text. These leaves tile the macro span exactly (losslessness).
    ROXYGEN_RD_MACRO_NAME,  // the `\name` head (backslash included)
    ROXYGEN_RD_MACRO_OPT,   // a `[...]` option group (e.g. `\link[pkg]{x}`)
    ROXYGEN_RD_MACRO_DELIM, // a `{` or `}` content delimiter
    ROXYGEN_RD_MACRO_VERB,  // verbatim content of a VERB macro (`\url`, `\verb`, …)
    // Roxygen logical-content nodes (the CST re-model). A block's children are
    // `ROXYGEN_SECTION`s (the intro prose, then one per `@tag`); a section's prose
    // is grouped into `ROXYGEN_PARAGRAPH`s between blank-line separators. Markers,
    // the marker→content whitespace, and inter-line newlines live as trivia leaves
    // threaded into the enclosing node.
    ROXYGEN_SECTION,
    ROXYGEN_PARAGRAPH,
    // Markdown inline leaves, emitted **only** under a resolved `@md` block mode
    // (Rd-first, markdown-second). Like the other protected-span leaves they hold
    // their whole span (delimiters included) so the run tiles exactly, but their
    // *kind* is what records the resolved mode: in non-markdown mode `*x*` and
    // `` `x` `` stay literal `ROXYGEN_TEXT`/`ROXYGEN_CODE`, so the CST (and the
    // projected Rd) differ by mode. The projector maps emphasis/strong to
    // `\emph`/`\strong`, and a code span to `\code` or `\verb` per roxygen2's
    // R-parseability rule. Appended here (not beside `ROXYGEN_CODE`) to keep the
    // earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_EMPH,
    ROXYGEN_MD_STRONG,
    ROXYGEN_MD_CODE,
    /// A markdown list item's leading marker (`-`/`*`/`+` or `1.`/`1)`), emitted
    /// **only** under a resolved `@md` block mode and only at a line's content
    /// start. Like the other markdown leaves it holds its literal source (the
    /// bullet/number punctuation, *without* the trailing space, so a marker that
    /// does not form a list — see the CommonMark interrupt rule — chunks for
    /// reflow exactly as the plain text it stands in for). The projector maps the
    /// enclosing `ROXYGEN_MD_LIST` to `\itemize`/`\enumerate` and each item to a
    /// name-only `\item`.
    ROXYGEN_MD_LIST_MARKER,
    // Markdown block-list nodes (resolved `@md` mode only). A `ROXYGEN_MD_LIST`
    // groups consecutive `ROXYGEN_MD_LIST_ITEM`s; each item owns its marker leaf
    // and inline content. `#'` markers, the marker→content whitespace, and
    // inter-line newlines are threaded in as trivia (losslessness), the way the
    // block Rd macros thread them.
    ROXYGEN_MD_LIST,
    ROXYGEN_MD_LIST_ITEM,
    /// A markdown image `![alt](url "title")`, emitted **only** under a resolved
    /// `@md` block mode. Like the other markdown leaves it holds its whole span
    /// (delimiters included) so the run tiles exactly. The projector maps it to
    /// `\figure{url}{title}`, wrapping the figure in `\if{html}{…}`/`\if{pdf}{…}`
    /// per roxygen2's extension-keyed `get_image_format` rule. Appended last to
    /// keep the earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_IMAGE,
    /// A markdown fenced code block (resolved `@md` mode only): a
    /// `ROXYGEN_MD_FENCE` opener leaf, the verbatim code lines, and a
    /// `ROXYGEN_MD_FENCE` closer leaf, with the `#'` markers, marker→content
    /// whitespace, and inter-line newlines threaded in as trivia (losslessness),
    /// the way the block Rd macros and markdown lists thread them. The projector
    /// maps it to roxygen2's `\if{html}{\out{<div…>}} \preformatted{…}
    /// \if{html}{\out{</div>}}` triple. Appended last to keep the earlier
    /// variants' `as u16` discriminants stable.
    ROXYGEN_MD_CODE_BLOCK,
    /// A fenced-code-block delimiter line (3+ backticks plus an optional info
    /// string), a leaf inside a `ROXYGEN_MD_CODE_BLOCK`.
    ROXYGEN_MD_FENCE,
    /// A raw inline-HTML tag (`<img …>`, `</span>`), emitted **only** under a
    /// resolved `@md` block mode. Like the other markdown leaves it holds its
    /// whole span so the run tiles exactly. The projector maps it to roxygen2's
    /// `\if{html}{\out{<tag>}}` (`mdxml_html_inline`). Appended last to keep the
    /// earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_HTML,
    /// A markdown **HTML block** (resolved `@md` mode only): a run of `#'` lines
    /// opened by a CommonMark HTML-block start (condition 6 — a block-level tag at
    /// line start) and continuing to the next blank line, with the `#'` markers,
    /// marker→content whitespace, and inter-line newlines threaded in as trivia
    /// (losslessness), the way the fenced code block threads them. Its line content
    /// is verbatim `ROXYGEN_TEXT`; the projector reconstructs the block text and
    /// maps it to roxygen2's `\if{html}{\out{…}}` (`mdxml_html_block`). Appended
    /// last to keep the earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_HTML_BLOCK,
    /// A raw markdown emphasis-delimiter run leaf (`*`, `**`, `_`, …), emitted
    /// only under a resolved `@md` block mode. The lexer carves the maximal same-
    /// char run neutrally; the inline pass (`parser::roxygen::inline`) resolves
    /// matched runs into `ROXYGEN_MD_EMPH`/`ROXYGEN_MD_STRONG` **nodes** (now node
    /// kinds, not leaves), whose opener/closer delimiters are themselves
    /// `ROXYGEN_MD_DELIM` leaves, and leaves an unmatched run as a literal
    /// `ROXYGEN_MD_DELIM` leaf (projected as plain text). Appended last to keep the
    /// earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_DELIM,
    /// A GFM **table** (resolved `@md` mode only): a header row, a delimiter row
    /// (`|---|:--:|`) whose cell count matches the header, and zero or more body
    /// rows, with the `#'` markers, marker→content whitespace, and inter-line
    /// newlines threaded in as trivia (losslessness), the way the fenced code block
    /// and HTML block thread them. Its line content is verbatim `ROXYGEN_TEXT` (and,
    /// for the delimiter row, whatever inline leaves the lexer carved); the
    /// projector reconstructs the rows and cells from the node text and maps them to
    /// roxygen2's `\tabular{<align>}{… \tab … \cr}`. Appended last to keep the
    /// earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_TABLE,
    /// A markdown ATX **heading** line (resolved `@md` mode only): a single line
    /// whose content is `#{1,6}` followed by a space/tab or the line end, holding
    /// the `#'` marker (trivia) and the verbatim `ROXYGEN_TEXT` heading leaf. The
    /// projector reads the level and title from the node text and hoists it into an
    /// Rd `\section` (level 1) / `\subsection` (level >= 2) out of the enclosing
    /// `@description`/`@details`. Appended last to keep the earlier variants'
    /// `as u16` discriminants stable.
    ROXYGEN_MD_HEADING,
    /// A markdown **block quote** (resolved `@md` mode only): a run of consecutive
    /// `#'` lines each opening with `>` (after up to three spaces), with the `#'`
    /// markers, marker→content whitespace, and inter-line newlines threaded in as
    /// trivia (losslessness), the way the HTML block threads them. Its line content
    /// is verbatim `ROXYGEN_TEXT`. roxygen2 does not support block quotes: it warns
    /// and renders the node's *flattened plain text* (`escape_comment(xml_text)`),
    /// which the projector reproduces (strip each `>` marker, flatten the inner
    /// markdown, concatenate with no separator). Appended last to keep the earlier
    /// variants' `as u16` discriminants stable.
    ROXYGEN_MD_BLOCK_QUOTE,
    /// A markdown **thematic break** (resolved `@md` mode only): a single line whose
    /// whole content is a CommonMark thematic break (`***`/`---`/`___`, three or more
    /// of one character with optional spaces between, up to three leading spaces),
    /// holding the `#'` marker (trivia) and the verbatim break leaf. roxygen2 has no
    /// thematic-break support: it warns and renders the empty `escape_comment(xml_text)`
    /// (a thematic break has no text), so the break contributes nothing and the
    /// surrounding paragraphs coalesce. The projector drops it in `section_body_parts`.
    /// Appended last to keep the earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_THEMATIC_BREAK,
    /// A markdown **indented code block** resolved under `@md` mode: a run of
    /// prose lines each indented four or more columns past roxygen2's stripped
    /// `#'` prefix (a CommonMark indented code block), with interior blank lines,
    /// holding the `#'` markers (trivia) and the verbatim code leaves. roxygen2
    /// renders it exactly like a fenced code block — the three-atom
    /// `\if{html}{\out{<div…>}}` / `\preformatted{…}` / `\if{html}{\out{</div>}}`
    /// sequence — but with a bare `sourceCode` class and each line's indentation
    /// stripped (see `serialize_md_indented_code`). Appended last to keep the
    /// earlier variants' `as u16` discriminants stable.
    ROXYGEN_MD_INDENTED_CODE,
}

impl SyntaxKind {
    /// Number of distinct kinds, sized to the last variant. Used to allocate
    /// dispatch tables indexed by `kind as usize` (see the linter's single-walk
    /// rule dispatch). Stays correct as long as `ROXYGEN_MD_INDENTED_CODE` remains
    /// the last variant.
    pub const COUNT: usize = SyntaxKind::ROXYGEN_MD_INDENTED_CODE as usize + 1;

    /// A roxygen line's bytes are carried by these leaf tokens, which stand in
    /// for the single `COMMENT` token a non-roxygen comment line uses.
    pub fn is_roxygen_token(self) -> bool {
        matches!(
            self,
            SyntaxKind::ROXYGEN_MARKER
                | SyntaxKind::ROXYGEN_AT
                | SyntaxKind::ROXYGEN_TAG_NAME
                | SyntaxKind::ROXYGEN_TAG_ARG
                | SyntaxKind::ROXYGEN_TEXT
                | SyntaxKind::ROXYGEN_CODE
                | SyntaxKind::ROXYGEN_RD_MACRO
                | SyntaxKind::ROXYGEN_MD_LINK
                | SyntaxKind::ROXYGEN_MD_CODE
                | SyntaxKind::ROXYGEN_MD_LIST_MARKER
                | SyntaxKind::ROXYGEN_MD_IMAGE
                | SyntaxKind::ROXYGEN_MD_FENCE
                | SyntaxKind::ROXYGEN_MD_HTML
                | SyntaxKind::ROXYGEN_MD_DELIM
        )
    }

    /// The prose / inline-markup roxygen content elements: plain text plus the
    /// protected spans (inline code, Rd macro, markdown link/code/list-marker/…)
    /// and the resolved emphasis/strong **nodes**. The `SyntaxKind`-side
    /// counterpart of [`crate::parser::lexer::RoxygenRole::Content`]; the single
    /// list the formatter derives "is this a prose element" from. Includes the
    /// `ROXYGEN_MD_EMPH`/`ROXYGEN_MD_STRONG` nodes (the inline pass's output) so the
    /// formatter treats a resolved span as one atomic prose chunk.
    pub fn is_roxygen_prose_content(self) -> bool {
        matches!(
            self,
            SyntaxKind::ROXYGEN_TEXT
                | SyntaxKind::ROXYGEN_CODE
                | SyntaxKind::ROXYGEN_RD_MACRO
                | SyntaxKind::ROXYGEN_MD_LINK
                | SyntaxKind::ROXYGEN_MD_EMPH
                | SyntaxKind::ROXYGEN_MD_STRONG
                | SyntaxKind::ROXYGEN_MD_CODE
                | SyntaxKind::ROXYGEN_MD_LIST_MARKER
                | SyntaxKind::ROXYGEN_MD_IMAGE
                | SyntaxKind::ROXYGEN_MD_FENCE
                | SyntaxKind::ROXYGEN_MD_HTML
                | SyntaxKind::ROXYGEN_MD_DELIM
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RLanguage {}

impl Language for RLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        match raw.0 {
            0 => SyntaxKind::ROOT,
            1 => SyntaxKind::BINARY_EXPR,
            2 => SyntaxKind::ASSIGNMENT_EXPR,
            3 => SyntaxKind::PAREN_EXPR,
            4 => SyntaxKind::IF_EXPR,
            5 => SyntaxKind::FOR_EXPR,
            6 => SyntaxKind::WHILE_EXPR,
            7 => SyntaxKind::REPEAT_EXPR,
            8 => SyntaxKind::FUNCTION_EXPR,
            9 => SyntaxKind::BLOCK_EXPR,
            10 => SyntaxKind::UNARY_EXPR,
            11 => SyntaxKind::IDENT,
            12 => SyntaxKind::INT,
            13 => SyntaxKind::FLOAT,
            14 => SyntaxKind::STRING,
            15 => SyntaxKind::COMMENT,
            16 => SyntaxKind::TILDE,
            17 => SyntaxKind::USER_OP,
            18 => SyntaxKind::LBRACK,
            19 => SyntaxKind::RBRACK,
            20 => SyntaxKind::LBRACK2,
            21 => SyntaxKind::RBRACK2,
            22 => SyntaxKind::PLUS,
            23 => SyntaxKind::MINUS,
            24 => SyntaxKind::STAR,
            25 => SyntaxKind::SLASH,
            26 => SyntaxKind::CARET,
            27 => SyntaxKind::PIPE,
            28 => SyntaxKind::COLON,
            29 => SyntaxKind::COLON2,
            30 => SyntaxKind::COLON3,
            31 => SyntaxKind::DOLLAR,
            32 => SyntaxKind::AT,
            33 => SyntaxKind::SEMICOLON,
            34 => SyntaxKind::COMMA,
            35 => SyntaxKind::OR,
            36 => SyntaxKind::OR2,
            37 => SyntaxKind::AND,
            38 => SyntaxKind::AND2,
            39 => SyntaxKind::EQUAL2,
            40 => SyntaxKind::NOT_EQUAL,
            41 => SyntaxKind::BANG,
            42 => SyntaxKind::LESS_THAN,
            43 => SyntaxKind::LESS_THAN_OR_EQUAL,
            44 => SyntaxKind::GREATER_THAN,
            45 => SyntaxKind::GREATER_THAN_OR_EQUAL,
            46 => SyntaxKind::LPAREN,
            47 => SyntaxKind::RPAREN,
            48 => SyntaxKind::IF_KW,
            49 => SyntaxKind::ELSE_KW,
            50 => SyntaxKind::FOR_KW,
            51 => SyntaxKind::WHILE_KW,
            52 => SyntaxKind::REPEAT_KW,
            53 => SyntaxKind::FUNCTION_KW,
            54 => SyntaxKind::IN_KW,
            55 => SyntaxKind::LBRACE,
            56 => SyntaxKind::RBRACE,
            57 => SyntaxKind::WHITESPACE,
            58 => SyntaxKind::NEWLINE,
            59 => SyntaxKind::ASSIGN_LEFT,
            60 => SyntaxKind::SUPER_ASSIGN,
            61 => SyntaxKind::ASSIGN_RIGHT,
            62 => SyntaxKind::SUPER_ASSIGN_RIGHT,
            63 => SyntaxKind::ASSIGN_EQ,
            64 => SyntaxKind::CALL_EXPR,
            65 => SyntaxKind::SUBSET_EXPR,
            66 => SyntaxKind::SUBSET2_EXPR,
            67 => SyntaxKind::ARG_LIST,
            68 => SyntaxKind::ARG,
            69 => SyntaxKind::ERROR,
            70 => SyntaxKind::COMPLEX,
            71 => SyntaxKind::QUESTION,
            72 => SyntaxKind::WALRUS,
            73 => SyntaxKind::ROXYGEN_MARKER,
            74 => SyntaxKind::ROXYGEN_AT,
            75 => SyntaxKind::ROXYGEN_TAG_NAME,
            76 => SyntaxKind::ROXYGEN_TAG_ARG,
            77 => SyntaxKind::ROXYGEN_TEXT,
            78 => SyntaxKind::ROXYGEN_CODE,
            79 => SyntaxKind::ROXYGEN_RD_MACRO,
            80 => SyntaxKind::ROXYGEN_MD_LINK,
            81 => SyntaxKind::ROXYGEN_BLOCK,
            82 => SyntaxKind::ROXYGEN_LINE,
            83 => SyntaxKind::ROXYGEN_TAG,
            84 => SyntaxKind::ROXYGEN_RD_MACRO_NAME,
            85 => SyntaxKind::ROXYGEN_RD_MACRO_OPT,
            86 => SyntaxKind::ROXYGEN_RD_MACRO_DELIM,
            87 => SyntaxKind::ROXYGEN_RD_MACRO_VERB,
            88 => SyntaxKind::ROXYGEN_SECTION,
            89 => SyntaxKind::ROXYGEN_PARAGRAPH,
            90 => SyntaxKind::ROXYGEN_MD_EMPH,
            91 => SyntaxKind::ROXYGEN_MD_STRONG,
            92 => SyntaxKind::ROXYGEN_MD_CODE,
            93 => SyntaxKind::ROXYGEN_MD_LIST_MARKER,
            94 => SyntaxKind::ROXYGEN_MD_LIST,
            95 => SyntaxKind::ROXYGEN_MD_LIST_ITEM,
            96 => SyntaxKind::ROXYGEN_MD_IMAGE,
            97 => SyntaxKind::ROXYGEN_MD_CODE_BLOCK,
            98 => SyntaxKind::ROXYGEN_MD_FENCE,
            99 => SyntaxKind::ROXYGEN_MD_HTML,
            100 => SyntaxKind::ROXYGEN_MD_HTML_BLOCK,
            101 => SyntaxKind::ROXYGEN_MD_DELIM,
            102 => SyntaxKind::ROXYGEN_MD_TABLE,
            103 => SyntaxKind::ROXYGEN_MD_HEADING,
            104 => SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE,
            105 => SyntaxKind::ROXYGEN_MD_THEMATIC_BREAK,
            106 => SyntaxKind::ROXYGEN_MD_INDENTED_CODE,
            _ => SyntaxKind::ERROR,
        }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<RLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<RLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<RLanguage>;
