use rowan::GreenNodeBuilder;

use crate::parser::events::Event;
use crate::parser::lexer::{TokKind, Token};
use crate::parser::roxygen::{
    is_verbatim_rd_arg, rd_backslash_is_escaped, rd_macro_arity, rd_macro_name_end, scan_balanced,
    scan_rd_macro,
};
use crate::syntax::{SyntaxKind, SyntaxNode};

pub(crate) fn build_tree(tokens: &[Token], events: &[Event]) -> SyntaxNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::ROOT.into());

    for event in events {
        match event {
            Event::Start(kind) => builder.start_node((*kind).into()),
            Event::Tok(idx) => push_token(&mut builder, &tokens[*idx]),
            Event::Leaf(kind, text) => builder.token((*kind).into(), text),
            Event::Finish => builder.finish_node(),
        }
    }

    builder.finish_node();
    let green = builder.finish();
    SyntaxNode::new_root(green)
}

fn push_token(builder: &mut GreenNodeBuilder<'_>, tok: &Token) {
    // An Rd macro is materialized as a *node* (not a leaf): its content is
    // sub-parsed so the CST models what `tools::parse_Rd` parses (nested macros
    // become child nodes), which the projector then translates faithfully.
    if matches!(tok.kind, TokKind::RoxygenRdMacro) {
        build_rd_macro(builder, tok.text);
    } else {
        builder.token(syntax_kind_for(&tok.kind).into(), tok.text);
    }
}

/// Where an Rd-macro expansion writes its nodes and leaves. The tree builder
/// writes green nodes directly; the roxygen block builder writes `Event`s (a
/// Form-B block macro's leading argument groups are expanded there, since the
/// node they belong to spans following `#'` lines). One expansion, two sinks —
/// so a nested `\code{x}` in an `\item` term is modeled identically whether the
/// call closed on its line or not.
pub(crate) trait RdSink {
    fn start(&mut self, kind: SyntaxKind);
    fn leaf(&mut self, kind: SyntaxKind, text: &str);
    fn finish(&mut self);
}

impl RdSink for GreenNodeBuilder<'_> {
    fn start(&mut self, kind: SyntaxKind) {
        self.start_node(kind.into());
    }
    fn leaf(&mut self, kind: SyntaxKind, text: &str) {
        self.token(kind.into(), text);
    }
    fn finish(&mut self) {
        self.finish_node();
    }
}

impl RdSink for Vec<Event> {
    fn start(&mut self, kind: SyntaxKind) {
        self.push(Event::Start(kind));
    }
    fn leaf(&mut self, kind: SyntaxKind, text: &str) {
        self.push(Event::Leaf(kind, text.to_string()));
    }
    fn finish(&mut self) {
        self.push(Event::Finish);
    }
}

/// Expand a `RoxygenRdMacro` token's text into a structured `ROXYGEN_RD_MACRO`
/// node, mirroring `tools::parse_Rd`: a `\name` head, an optional `[…]` option,
/// `{`/`}` delimiters, and content that is either verbatim (`VERB` macros, e.g.
/// `\url`) or sub-parsed so nested `\macro` calls become child nodes. The emitted
/// leaves tile `text` exactly (losslessness). `text` is a complete, well-formed
/// macro span — the lexer only produces the token when `scan_rd_macro` succeeded.
fn build_rd_macro<S: RdSink + ?Sized>(builder: &mut S, text: &str) {
    builder.start(SyntaxKind::ROXYGEN_RD_MACRO);
    let bytes = text.as_bytes();

    // `\name` (backslash plus the `[A-Za-z][A-Za-z0-9]*` run after it).
    let mut j = rd_macro_name_end(bytes, 1);
    builder.leaf(SyntaxKind::ROXYGEN_RD_MACRO_NAME, &text[..j]);
    let name = &text[1..j];

    // Optional `[…]` option group (e.g. the `[pkg]` in `\link[pkg]{x}`).
    if bytes.get(j) == Some(&b'[') {
        let opt_end = scan_balanced(bytes, j, b'[', b']').unwrap_or(bytes.len());
        builder.leaf(SyntaxKind::ROXYGEN_RD_MACRO_OPT, &text[j..opt_end]);
        j = opt_end;
    }

    // Each `{…}` argument group becomes a `{` DELIM, sub-parsed (or verbatim)
    // content, and a `}` DELIM. A multi-argument macro (`\item{term}{desc}`,
    // `\ifelse{fmt}{yes}{no}`) has further adjacent groups, up to its arity;
    // every other macro stops after the first. The group ends are found by
    // scanning, so the slices tile `text` exactly.
    let mut arg_index = 0;
    while bytes.get(j) == Some(&b'{') {
        let Some(group_end) = scan_balanced(bytes, j, b'{', b'}') else {
            break; // unbalanced: fall through to the defensive remainder
        };
        builder.leaf(SyntaxKind::ROXYGEN_RD_MACRO_DELIM, "{");
        let content = &text[j + 1..group_end - 1];
        // Verbatim is per *argument*, not per macro: `\href`'s first arg (the URL)
        // is `VERB` while its second (the link text) is sub-parsed like any
        // latexlike body.
        if is_verbatim_rd_arg(name, arg_index) {
            if !content.is_empty() {
                builder.leaf(SyntaxKind::ROXYGEN_RD_MACRO_VERB, content);
            }
        } else {
            build_rd_content(builder, content);
        }
        builder.leaf(SyntaxKind::ROXYGEN_RD_MACRO_DELIM, "}");
        j = group_end;
        arg_index += 1;
        if arg_index >= rd_macro_arity(name) {
            break;
        }
    }
    if j < text.len() {
        // Defensive: a span without the expected brace (or an unbalanced one)
        // keeps its remainder whole so the round-trip is preserved (the lexer
        // should never emit this shape for a well-formed macro).
        builder.leaf(SyntaxKind::ROXYGEN_TEXT, &text[j..]);
    }

    builder.finish();
}

/// Sub-parse the content of a latexlike Rd macro into alternating `ROXYGEN_TEXT`
/// runs and nested `ROXYGEN_RD_MACRO` nodes. Only a `\macro` call is structural;
/// everything else (including `\}` escapes and stray backslashes) is literal text.
///
/// A `\` that is itself escaped never begins a macro: parse_Rd pairs backslashes
/// left-to-right inside a braced argument exactly as it does in prose, so a `\`
/// preceded by an odd-length backslash run is consumed by its pair
/// ([`rd_backslash_is_escaped`]). `\\y` is a literal `\` + `y`, not a `\y` macro;
/// `\\\dots` re-forms `\dots` (the third backslash is unescaped).
pub(crate) fn build_rd_content<S: RdSink + ?Sized>(builder: &mut S, content: &str) {
    let bytes = content.as_bytes();
    let mut run_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && !rd_backslash_is_escaped(bytes, i)
            && let Some(end) = scan_rd_macro(bytes, i)
        {
            if run_start < i {
                builder.leaf(SyntaxKind::ROXYGEN_TEXT, &content[run_start..i]);
            }
            build_rd_macro(builder, &content[i..end]);
            i = end;
            run_start = i;
        } else {
            // `\` is ASCII, so advancing one byte keeps `run_start`/`i` on char
            // boundaries (we only ever slice at a `\` or the ends).
            i += 1;
        }
    }
    if run_start < bytes.len() {
        builder.leaf(SyntaxKind::ROXYGEN_TEXT, &content[run_start..]);
    }
}
/// The `SyntaxKind` a lexed token of `kind` is materialized as in the CST. The
/// single source of truth for the token-kind mapping, shared by [`build_tree`]
/// and incremental reparse (`crate::parser::reparse`).
pub(crate) fn syntax_kind_for(kind: &TokKind) -> SyntaxKind {
    match kind {
        TokKind::Ident => SyntaxKind::IDENT,
        TokKind::Int => SyntaxKind::INT,
        TokKind::Float => SyntaxKind::FLOAT,
        TokKind::Complex => SyntaxKind::COMPLEX,
        TokKind::String => SyntaxKind::STRING,
        TokKind::Comment => SyntaxKind::COMMENT,
        TokKind::Tilde => SyntaxKind::TILDE,
        TokKind::Question => SyntaxKind::QUESTION,
        TokKind::UserOp => SyntaxKind::USER_OP,
        TokKind::LBrack => SyntaxKind::LBRACK,
        TokKind::RBrack => SyntaxKind::RBRACK,
        TokKind::LBrack2 => SyntaxKind::LBRACK2,
        TokKind::RBrack2 => SyntaxKind::RBRACK2,
        TokKind::Plus => SyntaxKind::PLUS,
        TokKind::Minus => SyntaxKind::MINUS,
        TokKind::Star => SyntaxKind::STAR,
        TokKind::Slash => SyntaxKind::SLASH,
        TokKind::Caret => SyntaxKind::CARET,
        TokKind::Pipe => SyntaxKind::PIPE,
        TokKind::Colon => SyntaxKind::COLON,
        TokKind::Colon2 => SyntaxKind::COLON2,
        TokKind::Colon3 => SyntaxKind::COLON3,
        TokKind::Dollar => SyntaxKind::DOLLAR,
        TokKind::At => SyntaxKind::AT,
        TokKind::Semicolon => SyntaxKind::SEMICOLON,
        TokKind::Comma => SyntaxKind::COMMA,
        TokKind::Or => SyntaxKind::OR,
        TokKind::Or2 => SyntaxKind::OR2,
        TokKind::And => SyntaxKind::AND,
        TokKind::And2 => SyntaxKind::AND2,
        TokKind::Equal2 => SyntaxKind::EQUAL2,
        TokKind::NotEqual => SyntaxKind::NOT_EQUAL,
        TokKind::Bang => SyntaxKind::BANG,
        TokKind::LessThan => SyntaxKind::LESS_THAN,
        TokKind::LessThanOrEqual => SyntaxKind::LESS_THAN_OR_EQUAL,
        TokKind::GreaterThan => SyntaxKind::GREATER_THAN,
        TokKind::GreaterThanOrEqual => SyntaxKind::GREATER_THAN_OR_EQUAL,
        TokKind::LParen => SyntaxKind::LPAREN,
        TokKind::RParen => SyntaxKind::RPAREN,
        TokKind::IfKw => SyntaxKind::IF_KW,
        TokKind::ElseKw => SyntaxKind::ELSE_KW,
        TokKind::ForKw => SyntaxKind::FOR_KW,
        TokKind::WhileKw => SyntaxKind::WHILE_KW,
        TokKind::RepeatKw => SyntaxKind::REPEAT_KW,
        TokKind::FunctionKw => SyntaxKind::FUNCTION_KW,
        TokKind::LambdaFn => SyntaxKind::FUNCTION_KW,
        TokKind::InKw => SyntaxKind::IN_KW,
        TokKind::LBrace => SyntaxKind::LBRACE,
        TokKind::RBrace => SyntaxKind::RBRACE,
        TokKind::AssignLeft => SyntaxKind::ASSIGN_LEFT,
        TokKind::SuperAssign => SyntaxKind::SUPER_ASSIGN,
        TokKind::AssignRight => SyntaxKind::ASSIGN_RIGHT,
        TokKind::SuperAssignRight => SyntaxKind::SUPER_ASSIGN_RIGHT,
        TokKind::AssignEq => SyntaxKind::ASSIGN_EQ,
        TokKind::Walrus => SyntaxKind::WALRUS,
        TokKind::Whitespace => SyntaxKind::WHITESPACE,
        TokKind::Newline => SyntaxKind::NEWLINE,
        TokKind::Unknown => SyntaxKind::ERROR,
        TokKind::RoxygenMarker => SyntaxKind::ROXYGEN_MARKER,
        TokKind::RoxygenAt => SyntaxKind::ROXYGEN_AT,
        TokKind::RoxygenTagName => SyntaxKind::ROXYGEN_TAG_NAME,
        TokKind::RoxygenTagArg => SyntaxKind::ROXYGEN_TAG_ARG,
        TokKind::RoxygenText => SyntaxKind::ROXYGEN_TEXT,
        TokKind::RoxygenCode => SyntaxKind::ROXYGEN_CODE,
        TokKind::RoxygenRdMacro => SyntaxKind::ROXYGEN_RD_MACRO,
        TokKind::RoxygenMdLink => SyntaxKind::ROXYGEN_MD_LINK,
        // An inline-link bracket leaf is consumed by the inline pass (assembled
        // into a `ROXYGEN_MD_LINK` node); it never reaches the builder as a bare
        // token. Map to the delimiter leaf as a defensive fallback.
        TokKind::RoxygenMdBracket => SyntaxKind::ROXYGEN_MD_DELIM,
        TokKind::RoxygenMdImage => SyntaxKind::ROXYGEN_MD_IMAGE,
        // A raw emphasis-delimiter run lexed under `@md`. The inline pass resolves
        // matched runs into `ROXYGEN_MD_EMPH`/`ROXYGEN_MD_STRONG` *nodes* (whose
        // opener/closer delimiter leaves are synthesized `Event::Leaf`s also tagged
        // `ROXYGEN_MD_DELIM`); an unmatched run stays this literal leaf.
        TokKind::RoxygenMdDelim => SyntaxKind::ROXYGEN_MD_DELIM,
        TokKind::RoxygenMdCode => SyntaxKind::ROXYGEN_MD_CODE,
        TokKind::RoxygenMdListMarker => SyntaxKind::ROXYGEN_MD_LIST_MARKER,
        TokKind::RoxygenMdFence => SyntaxKind::ROXYGEN_MD_FENCE,
        TokKind::RoxygenMdHtml => SyntaxKind::ROXYGEN_MD_HTML,
        TokKind::RoxygenMdHtmlBlock => SyntaxKind::ROXYGEN_TEXT,
        TokKind::RoxygenMdTableDelim => SyntaxKind::ROXYGEN_TEXT,
        // An ATX heading line is verbatim text inside a `ROXYGEN_MD_HEADING` node
        // (the only path the grouper produces). Mapping the bare leaf to
        // `ROXYGEN_TEXT` keeps any unwrapped heading leaf as literal prose.
        TokKind::RoxygenMdHeading => SyntaxKind::ROXYGEN_TEXT,
        // A setext heading underline (`===`/`---`) is verbatim text: it lives inside
        // a `ROXYGEN_MD_HEADING` node when it promotes a preceding paragraph, and
        // inside a `ROXYGEN_MD_THEMATIC_BREAK` node when a dash run heads nothing.
        TokKind::RoxygenMdSetextUnderline => SyntaxKind::ROXYGEN_TEXT,
        // A block-quote line (`> quoted`) is verbatim text: it lives inside a
        // `ROXYGEN_MD_BLOCK_QUOTE` node once the builder gathers consecutive quote
        // lines; a leaf that is never gathered stays literal prose.
        TokKind::RoxygenMdBlockQuote => SyntaxKind::ROXYGEN_TEXT,
        // A thematic-break line (`***`/`___`/spaced forms) is verbatim text: it lives
        // inside a `ROXYGEN_MD_THEMATIC_BREAK` node; a leaf never gathered stays prose.
        TokKind::RoxygenMdThematicBreak => SyntaxKind::ROXYGEN_TEXT,
    }
}
