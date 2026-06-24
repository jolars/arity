use rowan::GreenNodeBuilder;

use crate::parser::events::Event;
use crate::parser::lexer::{TokKind, Token};
use crate::parser::roxygen::{
    is_two_arg_rd_macro, is_verbatim_rd_arg, rd_macro_name_end, scan_balanced, scan_rd_macro,
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
        build_rd_macro(builder, &tok.text);
    } else {
        builder.token(syntax_kind_for(&tok.kind).into(), tok.text.as_str());
    }
}

/// Expand a `RoxygenRdMacro` token's text into a structured `ROXYGEN_RD_MACRO`
/// node, mirroring `tools::parse_Rd`: a `\name` head, an optional `[…]` option,
/// `{`/`}` delimiters, and content that is either verbatim (`VERB` macros, e.g.
/// `\url`) or sub-parsed so nested `\macro` calls become child nodes. The emitted
/// leaves tile `text` exactly (losslessness). `text` is a complete, well-formed
/// macro span — the lexer only produces the token when `scan_rd_macro` succeeded.
fn build_rd_macro(builder: &mut GreenNodeBuilder<'_>, text: &str) {
    builder.start_node(SyntaxKind::ROXYGEN_RD_MACRO.into());
    let bytes = text.as_bytes();

    // `\name` (backslash plus the `[A-Za-z][A-Za-z0-9]*` run after it).
    let mut j = rd_macro_name_end(bytes, 1);
    builder.token(SyntaxKind::ROXYGEN_RD_MACRO_NAME.into(), &text[..j]);
    let name = &text[1..j];

    // Optional `[…]` option group (e.g. the `[pkg]` in `\link[pkg]{x}`).
    if bytes.get(j) == Some(&b'[') {
        let opt_end = scan_balanced(bytes, j, b'[', b']').unwrap_or(bytes.len());
        builder.token(SyntaxKind::ROXYGEN_RD_MACRO_OPT.into(), &text[j..opt_end]);
        j = opt_end;
    }

    // Each `{…}` argument group becomes a `{` DELIM, sub-parsed (or verbatim)
    // content, and a `}` DELIM. A two-argument macro (`\item{term}{desc}`) has a
    // second adjacent group; every other macro stops after the first. The group
    // ends are found by scanning, so the slices tile `text` exactly.
    let mut arg_index = 0;
    while bytes.get(j) == Some(&b'{') {
        let Some(group_end) = scan_balanced(bytes, j, b'{', b'}') else {
            break; // unbalanced: fall through to the defensive remainder
        };
        builder.token(SyntaxKind::ROXYGEN_RD_MACRO_DELIM.into(), "{");
        let content = &text[j + 1..group_end - 1];
        // Verbatim is per *argument*, not per macro: `\href`'s first arg (the URL)
        // is `VERB` while its second (the link text) is sub-parsed like any
        // latexlike body.
        if is_verbatim_rd_arg(name, arg_index) {
            if !content.is_empty() {
                builder.token(SyntaxKind::ROXYGEN_RD_MACRO_VERB.into(), content);
            }
        } else {
            build_rd_content(builder, content);
        }
        builder.token(SyntaxKind::ROXYGEN_RD_MACRO_DELIM.into(), "}");
        j = group_end;
        arg_index += 1;
        if !is_two_arg_rd_macro(name) {
            break;
        }
    }
    if j < text.len() {
        // Defensive: a span without the expected brace (or an unbalanced one)
        // keeps its remainder whole so the round-trip is preserved (the lexer
        // should never emit this shape for a well-formed macro).
        builder.token(SyntaxKind::ROXYGEN_TEXT.into(), &text[j..]);
    }

    builder.finish_node();
}

/// Sub-parse the content of a latexlike Rd macro into alternating `ROXYGEN_TEXT`
/// runs and nested `ROXYGEN_RD_MACRO` nodes. Only a `\macro` call is structural;
/// everything else (including `\}` escapes and stray backslashes) is literal text.
fn build_rd_content(builder: &mut GreenNodeBuilder<'_>, content: &str) {
    let bytes = content.as_bytes();
    let mut run_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some(end) = scan_rd_macro(bytes, i)
        {
            if run_start < i {
                builder.token(SyntaxKind::ROXYGEN_TEXT.into(), &content[run_start..i]);
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
        builder.token(SyntaxKind::ROXYGEN_TEXT.into(), &content[run_start..]);
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
        TokKind::RoxygenMdImage => SyntaxKind::ROXYGEN_MD_IMAGE,
        TokKind::RoxygenMdEmph => SyntaxKind::ROXYGEN_MD_EMPH,
        TokKind::RoxygenMdStrong => SyntaxKind::ROXYGEN_MD_STRONG,
        TokKind::RoxygenMdCode => SyntaxKind::ROXYGEN_MD_CODE,
        TokKind::RoxygenMdListMarker => SyntaxKind::ROXYGEN_MD_LIST_MARKER,
        TokKind::RoxygenMdFence => SyntaxKind::ROXYGEN_MD_FENCE,
        TokKind::RoxygenMdHtml => SyntaxKind::ROXYGEN_MD_HTML,
        // The HTML-block opener line is verbatim text inside a
        // `ROXYGEN_MD_HTML_BLOCK`; the block node is built by the grouping phase.
        TokKind::RoxygenMdHtmlBlock => SyntaxKind::ROXYGEN_TEXT,
    }
}
