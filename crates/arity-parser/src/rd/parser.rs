//! A mode-aware source scanner and recoverable recursive-descent Rd parser.
//!
//! Every token is an untouched slice at the current byte cursor. Recovery never
//! synthesizes source text, and the nesting guard retains the remaining bytes.

use rowan::{GreenNodeBuilder, ast::AstNode};

use super::ast::Document;
use super::macros::{self, Items, Mode};
use super::syntax::{SyntaxKind as K, SyntaxNode};
use crate::parser::ParseDiagnostic;

/// A complete source tree and recoverable, byte-ranged syntax diagnostics.
#[derive(Debug, Clone)]
pub struct ParseOutput {
    pub cst: SyntaxNode,
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl ParseOutput {
    /// Navigate the root through typed source wrappers.
    pub fn document(&self) -> Document {
        Document::cast(self.cst.clone()).expect("Rd parses always have a ROOT")
    }
}

/// Parse a complete UTF-8 Rd document without expanding or evaluating macros.
pub fn parse(text: &str) -> ParseOutput {
    let mut parser = Parser {
        text,
        pos: 0,
        builder: GreenNodeBuilder::new(),
        diagnostics: Vec::new(),
    };
    parser.builder.start_node(K::ROOT.into());
    parser.sequence(
        Context {
            mode: Mode::Prose,
            items: Items::None,
            sections: true,
        },
        Stop::Eof,
        &mut State::default(),
        0,
    );
    parser.builder.finish_node();
    parser
        .diagnostics
        .sort_by_key(|diagnostic| (diagnostic.start, diagnostic.end));
    ParseOutput {
        cst: SyntaxNode::new_root(parser.builder.finish()),
        diagnostics: parser.diagnostics,
    }
}

/// Reconstruct every source byte, including malformed input and line endings.
pub fn reconstruct(text: &str) -> String {
    parse(text).cst.text().to_string()
}

#[derive(Clone, Copy)]
struct Context {
    mode: Mode,
    items: Items,
    sections: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Eof,
    Brace,
    Bracket,
    Endif,
}

#[derive(Default)]
struct State {
    quote: Option<(u8, usize)>,
    braces: usize,
    r_comment: bool,
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl Parser<'_> {
    fn byte(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn emit(&mut self, kind: K, end: usize) {
        if end > self.pos {
            self.builder.token(kind.into(), &self.text[self.pos..end]);
            self.pos = end;
        }
    }

    fn diagnostic(&mut self, message: &str, start: usize, end: usize) {
        self.diagnostics.push(ParseDiagnostic {
            message: message.to_owned(),
            start,
            end,
        });
    }

    fn line_end(&self) -> usize {
        self.text[self.pos..]
            .find('\n')
            .map_or(self.text.len(), |offset| self.pos + offset)
    }

    fn directive(&self) -> Option<&'static str> {
        if self.pos != 0 && self.text.as_bytes()[self.pos - 1] != b'\n' {
            return None;
        }
        ["#ifdef", "#ifndef", "#endif"].into_iter().find(|name| {
            self.text[self.pos..].starts_with(name)
                && self
                    .text
                    .as_bytes()
                    .get(self.pos + name.len())
                    .is_none_or(|byte| !byte.is_ascii_alphabetic())
        })
    }

    fn leaf(mode: Mode) -> K {
        match mode {
            Mode::Prose => K::TEXT,
            Mode::RLike => K::RCODE,
            Mode::Verbatim | Mode::Equation => K::VERBATIM,
        }
    }

    fn sequence(&mut self, context: Context, stop: Stop, state: &mut State, depth: usize) {
        // This bounds Rust and rowan recursion on hostile input while retaining
        // a complete tree, rather than turning a resource limit into a panic.
        if depth >= 128 {
            self.diagnostic("Rd nesting limit exceeded", self.pos, self.pos);
            self.emit(K::ERROR, self.text.len());
            return;
        }
        while let Some(byte) = self.byte() {
            if let Some(name) = self.directive().filter(|_| context.mode != Mode::Equation) {
                if name == "#endif" {
                    if stop == Stop::Endif {
                        return;
                    }
                    self.diagnostic("unmatched #endif", self.pos, self.pos + name.len());
                    self.emit(K::DIRECTIVE, self.line_end());
                } else {
                    self.conditional(context, state, depth + 1, name);
                }
                continue;
            }
            if byte == b'%' && context.mode != Mode::Equation && !state.r_comment {
                let end = self.line_end();
                let end = if self.text.as_bytes()[end - 1] == b'\r' {
                    end - 1
                } else {
                    end
                };
                self.emit(K::COMMENT, end);
                continue;
            }
            if byte == b'}' && state.quote.is_none() && state.braces == 0 {
                if stop == Stop::Brace {
                    return;
                }
                // An unclosed conditional must not consume its parent's brace.
                if stop == Stop::Endif {
                    return;
                }
                self.diagnostic("unexpected closing brace", self.pos, self.pos + 1);
                self.emit(K::RBRACE, self.pos + 1);
                continue;
            }
            if byte == b']' && stop == Stop::Bracket {
                return;
            }
            if byte == b'{' && context.mode == Mode::Prose {
                if context.sections {
                    self.diagnostic(
                        "unexpected brace group at document level",
                        self.pos,
                        self.pos + 1,
                    );
                }
                self.group(K::GROUP, context, depth + 1);
                continue;
            }
            if byte == b'\\' {
                let next = self.text.as_bytes().get(self.pos + 1).copied();
                let markup = match context.mode {
                    Mode::Prose => next.is_some_and(|byte| byte.is_ascii_alphabetic()),
                    Mode::RLike if !state.r_comment => next.is_some_and(|byte| {
                        byte.is_ascii_alphabetic()
                            && (state.quote.is_none() || matches!(byte, b'l' | b'v'))
                    }),
                    _ => false,
                };
                if markup {
                    self.macro_call(context, depth + 1);
                    continue;
                }
                if self.escape(context, state) {
                    continue;
                }
            }
            if context.mode == Mode::RLike
                && state.quote.is_none()
                && !state.r_comment
                && matches!(byte, b'r' | b'R')
                && self.raw_string()
            {
                continue;
            }
            self.text_run(context, stop, state);
        }
        if let Some((_, start)) = state.quote.take() {
            self.diagnostic("unterminated R-like string", start, self.pos);
        }
    }

    fn text_run(&mut self, context: Context, stop: Stop, state: &mut State) {
        let start = self.pos;
        let mut cursor = start;
        let bytes = self.text.as_bytes();
        while let Some(&byte) = bytes.get(cursor) {
            if cursor > start
                && (byte == b'\\'
                    || (byte == b'%' && context.mode != Mode::Equation && !state.r_comment)
                    || (byte == b'{' && context.mode == Mode::Prose)
                    || (byte == b'}' && state.quote.is_none() && state.braces == 0)
                    || (byte == b']' && stop == Stop::Bracket)
                    || (byte == b'#' && bytes[cursor - 1] == b'\n')
                    || (context.mode == Mode::RLike
                        && state.quote.is_none()
                        && !state.r_comment
                        && matches!(byte, b'r' | b'R')
                        && matches!(bytes.get(cursor + 1), Some(b'\'' | b'"'))))
            {
                break;
            }
            if context.mode == Mode::RLike {
                if byte == b'\n' {
                    state.r_comment = false;
                }
                if !state.r_comment {
                    if let Some((quote, _)) = state.quote {
                        if byte == quote {
                            state.quote = None;
                        }
                    } else if matches!(byte, b'\'' | b'"' | b'`') {
                        state.quote = Some((byte, cursor));
                    } else if byte == b'#' {
                        state.r_comment = true;
                    }
                }
            }
            if state.quote.is_none() && context.mode != Mode::Prose {
                if byte == b'{' {
                    state.braces += 1;
                }
                if byte == b'}' {
                    state.braces = state.braces.saturating_sub(1);
                }
            }
            cursor += 1;
            // Physical newlines are a useful scanner boundary and keep raw
            // string and conditional recognition independent of surrounding text.
            if byte == b'\n' {
                break;
            }
        }
        self.emit(Self::leaf(context.mode), cursor);
    }

    fn escape(&mut self, context: Context, state: &mut State) -> bool {
        let bytes = self.text.as_bytes();
        let Some(&next) = bytes.get(self.pos + 1) else {
            return false;
        };
        if context.mode == Mode::RLike
            && let Some((quote, _)) = state.quote
        {
            let count = bytes[self.pos..]
                .iter()
                .take_while(|&&byte| byte == b'\\')
                .count();
            if bytes.get(self.pos + count) == Some(&quote) {
                if count % 4 == 0 {
                    state.quote = None;
                }
                self.emit(K::R_STRING_ESCAPE, self.pos + count + 1);
                return true;
            }
            // Consume a whole run once; rescanning it for each pair would
            // make long quoted backslash runs quadratic.
            if count > 1 {
                for _ in 0..count / 2 {
                    self.emit(K::ESCAPED_RCODE, self.pos + 2);
                }
                return true;
            }
        }
        let decoded = matches!(next, b'\\' | b'%')
            || (matches!(next, b'{' | b'}')
                && (context.mode != Mode::RLike || state.quote.is_none()));
        if decoded {
            let kind = match context.mode {
                Mode::Prose => K::ESCAPED_TEXT,
                Mode::RLike => K::ESCAPED_RCODE,
                Mode::Verbatim => K::ESCAPED_VERBATIM,
                Mode::Equation => K::VERBATIM,
            };
            self.emit(kind, self.pos + 2);
            return true;
        }
        // Unrecognized escapes in R strings and verbatim arguments retain both
        // characters. Consuming the pair also protects an escaped quote.
        if context.mode != Mode::Prose {
            let width = self.text[self.pos + 1..].chars().next().unwrap().len_utf8();
            if next == b'\n' {
                state.r_comment = false;
            }
            self.emit(Self::leaf(context.mode), self.pos + 1 + width);
            return true;
        }
        false
    }

    fn raw_string(&mut self) -> bool {
        let bytes = self.text.as_bytes();
        let Some(&quote @ (b'\'' | b'"')) = bytes.get(self.pos + 1) else {
            return false;
        };
        let mut opener = self.pos + 2;
        while bytes.get(opener) == Some(&b'-') {
            opener += 1;
        }
        let close = match bytes.get(opener) {
            Some(b'(') => ')',
            Some(b'[') => ']',
            Some(b'{') => '}',
            Some(b'|') => '|',
            _ => return false,
        };
        let ending = format!(
            "{close}{}{}",
            "-".repeat(opener - self.pos - 2),
            char::from(quote)
        );
        let end = if let Some(offset) = self.text[opener + 1..].find(&ending) {
            opener + 1 + offset + ending.len()
        } else {
            self.diagnostic("unterminated R-like raw string", self.pos, self.text.len());
            self.text.len()
        };
        self.emit(K::RCODE, end);
        true
    }

    fn group(&mut self, kind: K, context: Context, depth: usize) {
        let start = self.pos;
        self.builder.start_node(kind.into());
        self.emit(K::LBRACE, self.pos + 1);
        self.sequence(context, Stop::Brace, &mut State::default(), depth);
        if self.byte() == Some(b'}') {
            self.emit(K::RBRACE, self.pos + 1);
        } else {
            self.diagnostic("unclosed brace", start, start + 1);
        }
        self.builder.finish_node();
    }

    fn macro_call(&mut self, context: Context, depth: usize) {
        let start = self.pos;
        let mut end = start + 1;
        while self
            .text
            .as_bytes()
            .get(end)
            .is_some_and(u8::is_ascii_alphanumeric)
        {
            end += 1;
        }
        let mut spec = macros::lookup(&self.text[start + 1..end], context.items);
        // R retries unknown alphanumeric names without a trailing digit run.
        if spec.is_none() {
            let trimmed =
                self.text[start + 1..end].trim_end_matches(|ch: char| ch.is_ascii_digit());
            if let Some(found) = macros::lookup(trimmed, context.items) {
                end = start + 1 + trimmed.len();
                spec = Some(found);
            }
        }
        self.builder.start_node(K::MACRO.into());
        self.emit(K::MACRO_NAME, end);
        if let Some(spec) = spec {
            if context.sections && !spec.top_level {
                self.diagnostic("unexpected inline macro at document level", start, end);
            }
            if spec.section && !context.sections {
                self.diagnostic(
                    "unexpected section header inside an Rd argument",
                    start,
                    end,
                );
            }
            if spec.option && self.byte() == Some(b'[') {
                self.option(depth);
            }
            for (index, &mode) in spec.modes.iter().enumerate() {
                if self.byte() == Some(b'{') {
                    self.group(
                        K::ARGUMENT,
                        Context {
                            mode,
                            items: spec.items,
                            sections: false,
                        },
                        depth,
                    );
                } else if index < spec.required {
                    self.diagnostic("expected a braced macro argument", self.pos, self.pos);
                    break;
                }
            }
        }
        self.builder.finish_node();
    }

    fn option(&mut self, depth: usize) {
        let start = self.pos;
        self.builder.start_node(K::OPTION.into());
        self.emit(K::LBRACKET, self.pos + 1);
        self.sequence(
            Context {
                mode: Mode::Prose,
                items: Items::None,
                sections: false,
            },
            Stop::Bracket,
            &mut State::default(),
            depth,
        );
        if self.byte() == Some(b']') {
            self.emit(K::RBRACKET, self.pos + 1);
        } else {
            self.diagnostic("unclosed macro option", start, start + 1);
        }
        self.builder.finish_node();
    }

    fn conditional(&mut self, context: Context, state: &mut State, depth: usize, name: &str) {
        let start = self.pos;
        self.builder.start_node(K::CONDITIONAL.into());
        self.emit(K::DIRECTIVE, self.pos + name.len());
        let end = self.line_end();
        if self.text[self.pos..end].trim().is_empty() {
            self.diagnostic("expected a conditional target", self.pos, end);
        }
        self.emit(
            K::TARGET,
            (end + usize::from(end < self.text.len())).min(self.text.len()),
        );
        self.sequence(context, Stop::Endif, state, depth);
        if self.directive() == Some("#endif") {
            self.emit(K::DIRECTIVE, self.line_end());
            if self.byte() == Some(b'\n') {
                self.emit(K::DIRECTIVE, self.pos + 1);
            }
        } else {
            self.diagnostic("unclosed Rd conditional", start, start + name.len());
        }
        self.builder.finish_node();
    }
}
