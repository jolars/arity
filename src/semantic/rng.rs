//! Runtime RNG globals established by statically resolved initialization calls.

use rowan::ast::AstNode as _;

use super::symbols::unbacktick;
use super::{IdentRef, SemanticModel};
use crate::ast::{BinaryExpr, BlockExpr, CallExpr};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

impl SemanticModel {
    /// Whether completed statements in this execution frame establish the RNG
    /// global before a read. The caller resolves bare callees against its local,
    /// project, and package environments. Explicit base qualification is static.
    ///
    /// Only unconditional `set.seed` statements establish the fact. Ordinary
    /// statements preserve it, but possible dynamic environment changes kill it.
    /// A closure's definition position says nothing about when its body runs,
    /// and initialization inside a lazy argument does not establish caller state.
    pub(crate) fn rng_initialized_before(
        &self,
        root: &SyntaxNode,
        ident: &IdentRef,
        resolves_to_base: impl Fn(&CallExpr) -> bool,
    ) -> bool {
        if unbacktick(&ident.name) != ".Random.seed" || ident.deferred {
            return false;
        }
        let mut element = root.covering_element(ident.range);
        while let Some(parent) = element.parent() {
            if matches!(parent.kind(), SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR) {
                let preceding: Vec<_> = parent
                    .children_with_tokens()
                    .take_while(|child| child != &element)
                    .collect();
                for previous in preceding.iter().rev() {
                    if let Some(initialized) = preceding_effect(previous, &resolves_to_base) {
                        return initialized;
                    }
                }
            }
            if parent.kind() == SyntaxKind::FUNCTION_EXPR {
                return false;
            }
            // An earlier argument or branch condition can invalidate the global
            // before this read in the same statement. Do not assume argument
            // forcing order, even when initialization precedes the statement.
            if !matches!(parent.kind(), SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR)
                && may_change_environment(&parent)
            {
                return false;
            }
            element = parent.into();
        }
        false
    }
}

/// `None` preserves the incoming fact, `Some` replaces it. Blocks execute their
/// statements in order. No other expression exports an initialization fact:
/// branches may not run, and arbitrary call arguments are lazy in R.
fn preceding_effect(
    element: &SyntaxElement,
    resolves_to_base: &impl Fn(&CallExpr) -> bool,
) -> Option<bool> {
    let node = element.as_node()?;
    if let Some(block) = BlockExpr::cast(node.clone()) {
        let statements: Vec<_> = block.statements().collect();
        return statements
            .iter()
            .rev()
            .find_map(|statement| preceding_effect(statement, resolves_to_base));
    }
    if let Some(call) = CallExpr::cast(node.clone())
        && call.callee_name().as_deref().map(unbacktick) == Some("set.seed")
    {
        let qualified = call
            .base()
            .and_then(SyntaxElement::into_node)
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access());
        let is_base = match qualified {
            Some(access) => unbacktick(&access.package) == "base",
            None => resolves_to_base(&call),
        };
        if is_base {
            return Some(true);
        }
    }
    may_change_environment(node).then_some(false)
}

/// Known dynamic writes invalidate the fact even in conditional/lazy code:
/// those expressions might execute. Function literals merely create closures.
fn may_change_environment(node: &SyntaxNode) -> bool {
    if node.kind() == SyntaxKind::FUNCTION_EXPR {
        return false;
    }
    if let Some(call) = CallExpr::cast(node.clone()) {
        match call.callee_name().as_deref().map(unbacktick) {
            Some("on.exit") => return false,
            Some(
                "rm" | "remove" | "assign" | "delayedAssign" | "makeActiveBinding" | "eval"
                | "evalq" | "source" | "sys.source" | "load" | "attach" | "detach" | "do.call",
            ) => return true,
            _ => {}
        }
    }
    node.children().any(|child| may_change_environment(&child))
}
