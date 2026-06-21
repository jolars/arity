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
    ROXYGEN_LINE,
    ROXYGEN_TAG,
}

impl SyntaxKind {
    /// Number of distinct kinds, sized to the last variant. Used to allocate
    /// dispatch tables indexed by `kind as usize` (see the linter's single-walk
    /// rule dispatch). Stays correct as long as `ROXYGEN_TAG` remains the last
    /// variant.
    pub const COUNT: usize = SyntaxKind::ROXYGEN_TAG as usize + 1;

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
