//! The [`Expr`] union over "any R expression", and the [`HasArgList`] shared
//! trait for the argument-bearing nodes.
//!
//! R's leaf expressions are **bare tokens**, not `LITERAL` nodes (`1 + 2` is
//! `BINARY_EXPR { INT, PLUS, INT }`; a bare name is an `IDENT`). So [`Expr`]
//! casts from a [`SyntaxElement`] (node *or* token) and carries both node
//! variants and token-atom variants — a single `match Expr::cast(el)` then
//! covers every operand a consumer meets, with no separate "is it an IDENT
//! token" arm.

use rowan::TextRange;
use rowan::ast::support;

use crate::ast::AstNode;
use crate::ast::nodes::{
    Arg, ArgList, AssignmentExpr, BinaryExpr, BlockExpr, CallExpr, ForExpr, FunctionExpr, IfExpr,
    ParenExpr, RepeatExpr, Subset2Expr, SubsetExpr, UnaryExpr, WhileExpr,
};
use crate::ast::tokens::{AstToken, ComplexLit, FloatLit, Ident, IntLit, StringLit};
use crate::syntax::{RLanguage, SyntaxElement, SyntaxKind, SyntaxNode};

/// Any R expression: a compound node (`BINARY_EXPR`, `CALL_EXPR`, …) or a leaf
/// atom token (a name or a literal).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Assignment(AssignmentExpr),
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Paren(ParenExpr),
    Call(CallExpr),
    Subset(SubsetExpr),
    Subset2(Subset2Expr),
    If(IfExpr),
    For(ForExpr),
    While(WhileExpr),
    Repeat(RepeatExpr),
    Function(FunctionExpr),
    Block(BlockExpr),
    /// A bare identifier — also covers R's special constants (`TRUE`, `NA`, …);
    /// call [`Ident::constant`] to distinguish.
    Name(Ident),
    IntLiteral(IntLit),
    FloatLiteral(FloatLit),
    StringLiteral(StringLit),
    ComplexLiteral(ComplexLit),
}

/// Dispatch `$body` over every variant, binding the inner wrapper to `$x`.
macro_rules! for_each_variant {
    ($self:ident, $x:ident => $body:expr) => {
        match $self {
            Expr::Assignment($x) => $body,
            Expr::Binary($x) => $body,
            Expr::Unary($x) => $body,
            Expr::Paren($x) => $body,
            Expr::Call($x) => $body,
            Expr::Subset($x) => $body,
            Expr::Subset2($x) => $body,
            Expr::If($x) => $body,
            Expr::For($x) => $body,
            Expr::While($x) => $body,
            Expr::Repeat($x) => $body,
            Expr::Function($x) => $body,
            Expr::Block($x) => $body,
            Expr::Name($x) => $body,
            Expr::IntLiteral($x) => $body,
            Expr::FloatLiteral($x) => $body,
            Expr::StringLiteral($x) => $body,
            Expr::ComplexLiteral($x) => $body,
        }
    };
}

impl Expr {
    /// Cast a CST element to an expression. A node dispatches on its kind; a
    /// token becomes an atom variant (name or literal). Trivia, operators,
    /// keywords, and punctuation are not expressions and yield `None`.
    pub fn cast(element: SyntaxElement) -> Option<Expr> {
        match element {
            SyntaxElement::Node(node) => Self::cast_node(node),
            SyntaxElement::Token(token) => Some(match token.kind() {
                SyntaxKind::IDENT => Expr::Name(Ident::cast(token)?),
                SyntaxKind::INT => Expr::IntLiteral(IntLit::cast(token)?),
                SyntaxKind::FLOAT => Expr::FloatLiteral(FloatLit::cast(token)?),
                SyntaxKind::STRING => Expr::StringLiteral(StringLit::cast(token)?),
                SyntaxKind::COMPLEX => Expr::ComplexLiteral(ComplexLit::cast(token)?),
                _ => return None,
            }),
        }
    }

    /// Cast a node to a compound expression. `None` for a non-expression node
    /// (e.g. `ARG_LIST`, `ROOT`) — a leaf atom is a token, so use [`Expr::cast`]
    /// when the element may be either.
    pub fn cast_node(node: SyntaxNode) -> Option<Expr> {
        Some(match node.kind() {
            SyntaxKind::ASSIGNMENT_EXPR => Expr::Assignment(AssignmentExpr::cast(node)?),
            SyntaxKind::BINARY_EXPR => Expr::Binary(BinaryExpr::cast(node)?),
            SyntaxKind::UNARY_EXPR => Expr::Unary(UnaryExpr::cast(node)?),
            SyntaxKind::PAREN_EXPR => Expr::Paren(ParenExpr::cast(node)?),
            SyntaxKind::CALL_EXPR => Expr::Call(CallExpr::cast(node)?),
            SyntaxKind::SUBSET_EXPR => Expr::Subset(SubsetExpr::cast(node)?),
            SyntaxKind::SUBSET2_EXPR => Expr::Subset2(Subset2Expr::cast(node)?),
            SyntaxKind::IF_EXPR => Expr::If(IfExpr::cast(node)?),
            SyntaxKind::FOR_EXPR => Expr::For(ForExpr::cast(node)?),
            SyntaxKind::WHILE_EXPR => Expr::While(WhileExpr::cast(node)?),
            SyntaxKind::REPEAT_EXPR => Expr::Repeat(RepeatExpr::cast(node)?),
            SyntaxKind::FUNCTION_EXPR => Expr::Function(FunctionExpr::cast(node)?),
            SyntaxKind::BLOCK_EXPR => Expr::Block(BlockExpr::cast(node)?),
            _ => return None,
        })
    }

    /// The underlying CST element (a node for compound expressions, a token for
    /// atoms).
    pub fn syntax(&self) -> SyntaxElement {
        for_each_variant!(self, x => x.syntax().clone().into())
    }

    /// The source range of the expression.
    pub fn text_range(&self) -> TextRange {
        self.syntax().text_range()
    }

    /// Whether this is a primary/atomic expression — one that can be prefixed
    /// with `!` (or dropped) without reparenthesizing. Namely a name, a literal,
    /// a call, a parenthesized expression, or an index (`[`/`[[`). This is the
    /// guard a negating rewrite (`x == FALSE` → `!x`) needs to stay correct.
    pub fn is_atom(&self) -> bool {
        matches!(
            self,
            Expr::Name(_)
                | Expr::IntLiteral(_)
                | Expr::FloatLiteral(_)
                | Expr::StringLiteral(_)
                | Expr::ComplexLiteral(_)
                | Expr::Call(_)
                | Expr::Paren(_)
                | Expr::Subset(_)
                | Expr::Subset2(_)
        )
    }
}

/// A node that owns an argument list: `CALL_EXPR` (`f(…)`), `SUBSET_EXPR`
/// (`x[…]`), and `SUBSET2_EXPR` (`x[[…]]`).
pub trait HasArgList: AstNode<Language = RLanguage> {
    /// The `ARG_LIST` child, if present.
    fn arg_list(&self) -> Option<ArgList> {
        support::child(self.syntax())
    }

    /// The arguments, each split into optional name and value via [`Arg`].
    fn args(&self) -> impl Iterator<Item = Arg> {
        self.arg_list()
            .map(|list| list.args().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
    }

    /// The value of the `n`th positional (unnamed) argument, 0-indexed.
    fn nth_positional(&self, n: usize) -> Option<SyntaxElement> {
        self.args()
            .filter(|arg| !arg.is_named())
            .nth(n)
            .and_then(|arg| arg.value())
    }

    /// The value of the argument named `name`, if present.
    fn named_arg(&self, name: &str) -> Option<SyntaxElement> {
        self.args()
            .find(|arg| arg.name().as_deref() == Some(name))
            .and_then(|arg| arg.value())
    }
}

impl HasArgList for CallExpr {}
impl HasArgList for SubsetExpr {}
impl HasArgList for Subset2Expr {}
