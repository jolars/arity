//! A cursor-free, serializable handle to a CST node.
//!
//! rowan ships [`SyntaxNodePtr`](rowan::ast::SyntaxNodePtr) /
//! [`AstPtr`](rowan::ast::AstPtr) for the same-revision case (re-exported from
//! [`crate::ast`]), but their fields are private — they cannot be constructed
//! from a mapped `(kind, range)` or deserialized. [`NodePtr`] is ravel's
//! canonical handle: it owns its construction (so a range mapped across an edit
//! can be re-resolved) and derives `serde` (so it can ride an LSP
//! `CallHierarchyItem.data` field).
//!
//! Like rowan's pointers, a `NodePtr` resolves only against a tree built from
//! the *same text* it was taken against. To survive an edit, first map its range
//! through that edit
//! ([`map_range_through_edit`](crate::parser::reparse::map_range_through_edit)),
//! then resolve against the new tree — the path
//! [`Analysis::resolve_ptr`](crate::incremental::Analysis::resolve_ptr) takes.

use rowan::{NodeOrToken, TextRange, TextSize};

use crate::syntax::{SyntaxKind, SyntaxNode};

/// A location-based handle to a syntax node: its [`SyntaxKind`] plus its text
/// range. Cheap (`Copy`), serializable, and resolvable to a live [`SyntaxNode`]
/// via [`NodePtr::try_to_node`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodePtr {
    kind: SyntaxKind,
    start: u32,
    len: u32,
}

impl NodePtr {
    /// Capture a handle to `node`.
    pub fn from_node(node: &SyntaxNode) -> Self {
        Self::new(node.kind(), node.text_range())
    }

    /// The kind of node this points to.
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// The text range this points to.
    pub fn text_range(&self) -> TextRange {
        TextRange::at(TextSize::new(self.start), TextSize::new(self.len))
    }

    /// Return a copy pointing at `range`, keeping the kind. Used after mapping
    /// the range across an edit.
    pub fn with_range(self, range: TextRange) -> Self {
        Self::new(self.kind, range)
    }

    /// Resolve this handle to a node in `root`, which must be a *root* built from
    /// the same text the handle was taken against (or a range already mapped onto
    /// that text). Returns `None` if `root` is not a root, the range falls
    /// outside it, or no node of the stored kind spans exactly the stored range.
    pub fn try_to_node(&self, root: &SyntaxNode) -> Option<SyntaxNode> {
        if root.parent().is_some() {
            return None;
        }
        let range = self.text_range();
        if !root.text_range().contains_range(range) {
            return None;
        }
        let start = match root.covering_element(range) {
            NodeOrToken::Node(node) => node,
            NodeOrToken::Token(token) => token.parent()?,
        };
        // `covering_element` returns the deepest element spanning `range`; the
        // node we want is it or an ancestor with the exact range and kind.
        start
            .ancestors()
            .find(|node| node.text_range() == range && node.kind() == self.kind)
    }

    fn new(kind: SyntaxKind, range: TextRange) -> Self {
        Self {
            kind,
            start: range.start().into(),
            len: range.len().into(),
        }
    }
}
