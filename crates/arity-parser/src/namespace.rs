//! A lossless, typed syntax surface for R package `NAMESPACE` files.
//!
//! R first parses a `NAMESPACE` file as R syntax, then interprets a deliberately
//! small set of calls, blocks, assignments, and conditional expressions. This
//! module mirrors that split: it reuses the lossless R CST and adds typed
//! namespace entries without evaluating conditions or assigning meaning to a
//! directive's arguments.
//!
//! ```
//! use arity_parser::namespace::{self, DirectiveKind};
//!
//! let source = "export(foo)\nimportFrom(stats, predict)\n";
//! let output = namespace::parse(source);
//! assert!(output.diagnostics.is_empty());
//! assert_eq!(
//!     output
//!         .document()
//!         .directives()
//!         .map(|directive| directive.kind())
//!         .collect::<Vec<_>>(),
//!     [DirectiveKind::Export, DirectiveKind::ImportFrom]
//! );
//! assert_eq!(namespace::reconstruct(source), source);
//! ```

use rowan::{SyntaxElement, SyntaxToken, TextRange};
use smol_str::SmolStr;

use crate::ast::{
    Arg, AssignmentExpr, AstNode, BlockExpr, CallExpr, HasArgList, IfExpr, Root, token_name,
};
use crate::parser;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

pub use crate::parser::ParseDiagnostic;

/// A lossless `NAMESPACE` parse and its recoverable diagnostics.
#[derive(Debug, Clone)]
pub struct ParseOutput {
    /// The ordinary R CST, preserved byte for byte.
    pub cst: SyntaxNode,
    /// R syntax and namespace-shape diagnostics, sorted by source position.
    pub diagnostics: Vec<ParseDiagnostic>,
}

impl ParseOutput {
    /// The typed `NAMESPACE` view of the parsed root.
    pub fn document(&self) -> Document {
        Document {
            root: Root::cast(self.cst.clone()).expect("the R parser always roots the tree at ROOT"),
        }
    }
}

/// Parse a `NAMESPACE` buffer without evaluating it.
///
/// The underlying R parser remains total and lossless. Calls in namespace
/// directive positions are retained even when unsupported; other expressions
/// in those positions become [`Malformed`] entries. Namespace diagnostics do
/// not replace or suppress ordinary R syntax diagnostics.
pub fn parse(text: &str) -> ParseOutput {
    let parsed = parser::parse(text);
    let document = Document {
        root: Root::cast(parsed.cst.clone()).expect("the R parser always roots the tree at ROOT"),
    };
    let mut diagnostics = parsed.diagnostics;

    for entry in document.entries() {
        match entry {
            Entry::Directive(directive) if directive.kind() == DirectiveKind::Unsupported => {
                let Some(name) = directive.name_token() else {
                    continue;
                };
                push_if_uncovered(
                    &mut diagnostics,
                    "unsupported NAMESPACE directive",
                    name.text_range(),
                );
            }
            Entry::Malformed(malformed) => push_if_uncovered(
                &mut diagnostics,
                "expected a NAMESPACE directive",
                malformed.text_range(),
            ),
            Entry::Directive(_) => {}
        }
    }

    diagnostics.sort_by_key(|diagnostic| (diagnostic.start, diagnostic.end));
    ParseOutput {
        cst: parsed.cst,
        diagnostics,
    }
}

/// Round-trip `text` through the namespace parser. Always equal to `text`.
pub fn reconstruct(text: &str) -> String {
    parse(text)
        .cst
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .map(|token| token.text().to_string())
        .collect()
}

fn push_if_uncovered(diagnostics: &mut Vec<ParseDiagnostic>, message: &str, range: TextRange) {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    let overlaps_existing = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.start < end && start < diagnostic.end);
    if !overlaps_existing {
        diagnostics.push(ParseDiagnostic {
            message: message.to_string(),
            start,
            end,
        });
    }
}

/// A typed view of a complete `NAMESPACE` document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Document {
    root: Root,
}

impl Document {
    /// The underlying lossless R root, including all trivia.
    pub fn syntax(&self) -> &SyntaxNode {
        self.root.syntax()
    }

    /// Directive and malformed entries in source order.
    ///
    /// Braced groups, assignments, and both conditional branches are traversed
    /// as namespace containers. Conditions themselves are never evaluated and
    /// calls inside a condition or directive argument are not promoted.
    pub fn entries(&self) -> impl Iterator<Item = Entry> + use<> {
        let mut entries = Vec::new();
        for element in self.root.syntax().children_with_tokens() {
            collect_entry(element, None, &mut entries);
        }
        entries.into_iter()
    }

    /// Every directive in source order, including unsupported directives.
    pub fn directives(&self) -> impl Iterator<Item = Directive> + use<> {
        self.entries().filter_map(|entry| match entry {
            Entry::Directive(directive) => Some(directive),
            Entry::Malformed(_) => None,
        })
    }
}

/// One flattened namespace entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Entry {
    /// A named call in a directive position.
    Directive(Directive),
    /// R syntax that is not a valid namespace container or directive.
    Malformed(Malformed),
}

/// A named call in a namespace directive position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Directive {
    call: CallExpr,
    range: TextRange,
}

impl Directive {
    /// The recognized directive kind, or [`DirectiveKind::Unsupported`].
    pub fn kind(&self) -> DirectiveKind {
        self.name()
            .as_deref()
            .map(DirectiveKind::from_name)
            .unwrap_or(DirectiveKind::Unsupported)
    }

    /// The directive name, with quotes removed when it was quoted.
    pub fn name(&self) -> Option<SmolStr> {
        self.name_token().as_ref().map(token_name)
    }

    /// The directive name token and its exact source range.
    pub fn name_token(&self) -> Option<SyntaxToken<RLanguage>> {
        direct_callee_token(&self.call)
    }

    /// The call node, including its arguments and internal trivia.
    pub fn syntax(&self) -> &SyntaxNode {
        self.call.syntax()
    }

    /// The full namespace expression's range.
    ///
    /// This equals the call range for an ordinary directive and includes the
    /// assignment wrapper for `dll <- useDynLib(...)`.
    pub fn text_range(&self) -> TextRange {
        self.range
    }

    /// The directive call's range, excluding an optional assignment wrapper.
    pub fn call_range(&self) -> TextRange {
        self.call.syntax().text_range()
    }

    /// The opening `(`, if recovery produced one.
    pub fn l_paren(&self) -> Option<SyntaxToken<RLanguage>> {
        child_token(self.call.syntax(), SyntaxKind::LPAREN)
    }

    /// The closing `)`, if recovery produced one.
    pub fn r_paren(&self) -> Option<SyntaxToken<RLanguage>> {
        child_token(self.call.syntax(), SyntaxKind::RPAREN)
    }

    /// Arguments in source order, including empty or malformed slots.
    pub fn arguments(&self) -> impl Iterator<Item = Argument> + '_ {
        self.call.args().map(|argument| Argument { argument })
    }
}

/// One directive argument, retaining its raw CST and exact source ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Argument {
    argument: Arg,
}

impl Argument {
    /// The underlying argument node, including internal trivia.
    pub fn syntax(&self) -> &SyntaxNode {
        self.argument.syntax()
    }

    /// The whole argument range, excluding its comma separator.
    pub fn text_range(&self) -> TextRange {
        self.argument.syntax().text_range()
    }

    /// The optional keyword name, unquoted.
    pub fn name(&self) -> Option<SmolStr> {
        self.argument.name()
    }

    /// The optional keyword name token and its exact source range.
    pub fn name_token(&self) -> Option<SyntaxToken<RLanguage>> {
        self.argument.name_token()
    }

    /// The argument's value syntax, whether a token atom or compound node.
    pub fn value(&self) -> Option<SyntaxElement<RLanguage>> {
        self.argument.value()
    }

    /// The exact range of [`Self::value`].
    pub fn value_range(&self) -> Option<TextRange> {
        self.value().map(|value| value.text_range())
    }
}

/// An expression that occupies a directive position but is not a directive or
/// supported namespace container.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Malformed {
    syntax: SyntaxElement<RLanguage>,
    range: TextRange,
}

impl Malformed {
    /// The retained R syntax.
    pub fn syntax(&self) -> &SyntaxElement<RLanguage> {
        &self.syntax
    }

    /// The invalid namespace expression's range.
    pub fn text_range(&self) -> TextRange {
        self.range
    }
}

/// R's recognized namespace directive names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectiveKind {
    Export,
    ExportPattern,
    ExportClassPattern,
    ExportClass,
    ExportClasses,
    ExportMethods,
    Import,
    ImportFrom,
    ImportClassFrom,
    ImportClassesFrom,
    ImportMethodsFrom,
    UseDynLib,
    S3Method,
    /// A named directive call that this parser version does not support.
    Unsupported,
}

impl DirectiveKind {
    /// Classify a directive name exactly as R does, including capitalization.
    pub fn from_name(name: &str) -> Self {
        match name {
            "export" => Self::Export,
            "exportPattern" => Self::ExportPattern,
            "exportClassPattern" => Self::ExportClassPattern,
            "exportClass" => Self::ExportClass,
            "exportClasses" => Self::ExportClasses,
            "exportMethods" => Self::ExportMethods,
            "import" => Self::Import,
            "importFrom" => Self::ImportFrom,
            "importClassFrom" => Self::ImportClassFrom,
            "importClassesFrom" => Self::ImportClassesFrom,
            "importMethodsFrom" => Self::ImportMethodsFrom,
            "useDynLib" => Self::UseDynLib,
            "S3method" => Self::S3Method,
            _ => Self::Unsupported,
        }
    }
}

fn child_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken<RLanguage>> {
    node.children_with_tokens()
        .find_map(|element| match element {
            SyntaxElement::Token(token) if token.kind() == kind => Some(token),
            _ => None,
        })
}

fn direct_callee_token(call: &CallExpr) -> Option<SyntaxToken<RLanguage>> {
    match call.base()? {
        SyntaxElement::Token(token)
            if matches!(token.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) =>
        {
            Some(token)
        }
        _ => None,
    }
}

fn collect_entry(
    element: SyntaxElement<RLanguage>,
    enclosing_range: Option<TextRange>,
    entries: &mut Vec<Entry>,
) {
    if is_namespace_trivia(&element) {
        return;
    }

    let range = enclosing_range.unwrap_or_else(|| element.text_range());
    let SyntaxElement::Node(node) = element.clone() else {
        entries.push(Entry::Malformed(Malformed {
            syntax: element,
            range,
        }));
        return;
    };

    match node.kind() {
        SyntaxKind::CALL_EXPR => {
            let call = CallExpr::cast(node).expect("CALL_EXPR always casts");
            if direct_callee_token(&call).is_some() {
                entries.push(Entry::Directive(Directive { call, range }));
            } else {
                entries.push(Entry::Malformed(Malformed {
                    syntax: element,
                    range,
                }));
            }
        }
        SyntaxKind::BLOCK_EXPR => {
            let block = BlockExpr::cast(node).expect("BLOCK_EXPR always casts");
            for statement in block.statements() {
                collect_entry(statement, enclosing_range, entries);
            }
        }
        SyntaxKind::IF_EXPR => {
            let conditional = IfExpr::cast(node).expect("IF_EXPR always casts");
            if let Some(then_elements) = conditional.then_elements() {
                for branch_element in then_elements {
                    collect_entry(branch_element, enclosing_range, entries);
                }
            }
            if let Some(else_elements) = conditional.else_elements() {
                for branch_element in else_elements {
                    collect_entry(branch_element, enclosing_range, entries);
                }
            }
        }
        SyntaxKind::ASSIGNMENT_EXPR => {
            let assignment = AssignmentExpr::cast(node).expect("ASSIGNMENT_EXPR always casts");
            if matches!(
                assignment.op_kind(),
                Some(SyntaxKind::ASSIGN_EQ | SyntaxKind::ASSIGN_LEFT)
            ) && let Some(value) = assignment.value_element()
            {
                collect_entry(value, Some(range), entries);
            } else {
                entries.push(Entry::Malformed(Malformed {
                    syntax: element,
                    range,
                }));
            }
        }
        // A `#'` line is still a comment in a NAMESPACE file. The shared R
        // parser gives it richer roxygen structure, which this typed view
        // deliberately treats as trivia rather than rejecting.
        SyntaxKind::ROXYGEN_BLOCK => {}
        _ => entries.push(Entry::Malformed(Malformed {
            syntax: element,
            range,
        })),
    }
}

fn is_namespace_trivia(element: &SyntaxElement<RLanguage>) -> bool {
    matches!(
        element.kind(),
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::SEMICOLON | SyntaxKind::COMMENT
    )
}
