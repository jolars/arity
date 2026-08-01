//! Per-region control-flow graph.
//!
//! A [`ControlFlowGraph`] is built for each **flow region** — every function
//! body plus the file top-level. Regions are the boundaries the flow-affecting
//! keywords respect: `return()`/`stop()` leave the enclosing *function*,
//! `break`/`next` the enclosing *loop*. A [`FileControlFlow`] bundles the
//! top-level region with one CFG per `FUNCTION_EXPR` (keyed by [`NodePtr`]), and
//! is what [`crate::incremental::control_flow`] memoizes per file.
//!
//! The graph is built by **structured recursive descent** over the AST wrappers
//! ([`IfExpr`], [`ForExpr`], [`WhileExpr`], [`RepeatExpr`],
//! [`BlockExpr`](crate::ast::BlockExpr), [`CallExpr`]) — deterministic and
//! local, no fixpoint. It is **purely
//! syntactic**: a terminator is recognized by callee name (`return`/`stop`) or
//! bare reserved word (`break`/`next`, which R lexes as identifiers), and no
//! [`SymbolProvider`](crate::semantic::SymbolProvider) is consulted. A consumer
//! that cares whether `return` is the *base* `return` (and not a local
//! redefinition) applies that confirmation itself — see [`always_diverges`] and
//! the `unreachable-code` rule.
//!
//! Reachability falls out of the construction: statements that can never run
//! land in a block whose terminator is [`Terminator::Unreachable`] (the tail
//! after an unconditional divergence), and a divergence in *both* arms of an
//! `if`/`else` propagates so the code after the `if` is unreachable too — the
//! signal the linter's `unreachable-code` rule needs.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::kinds::is_trivia;
use crate::ast::{CallExpr, ForExpr, FunctionExpr, IfExpr, RepeatExpr, WhileExpr};
use crate::syntax::{NodePtr, SyntaxElement, SyntaxKind, SyntaxNode};

/// Index of a [`BasicBlock`] within a [`ControlFlowGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u32);

impl BlockId {
    /// The block's numeric index (for rendering and lookups).
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A maximal straight-line run of statements ending in a single [`Terminator`].
/// `stmts` are the source ranges of the statements executed, in order (a bare
/// token statement like `2` is included by its range, so this covers node *and*
/// token statements).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub stmts: Vec<TextRange>,
    pub terminator: Terminator,
}

/// How control leaves a [`BasicBlock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    /// Unconditional edge to a successor (fallthrough, loop back-edge, `break`,
    /// `next`).
    Goto(BlockId),
    /// Two-way branch (an `if`, or a `for`/`while` header that may skip the body).
    Branch {
        then_blk: BlockId,
        else_blk: BlockId,
    },
    /// Falls off the end of the region — a normal return to the caller.
    Return,
    /// Diverges out of the region via `return()`/`stop()`; no in-region successor.
    Diverge,
    /// The block is unreachable (it has no predecessor): the tail after an
    /// unconditional divergence.
    Unreachable,
}

/// The control-flow graph of a single region (a function body or the file
/// top-level). Block 0 is always the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    entry: BlockId,
}

impl Default for ControlFlowGraph {
    fn default() -> Self {
        Self {
            blocks: vec![BasicBlock {
                stmts: Vec::new(),
                terminator: Terminator::Return,
            }],
            entry: BlockId(0),
        }
    }
}

impl ControlFlowGraph {
    /// The graph's basic blocks, block 0 first.
    pub fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }

    /// The entry block.
    pub fn entry(&self) -> BlockId {
        self.entry
    }

    /// Look up a block by id.
    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.0 as usize]
    }

    /// Build the CFG for a region given its ordered statement elements.
    fn build_region(stmts: &[SyntaxElement]) -> Self {
        let mut builder = Builder { blocks: Vec::new() };
        let entry = builder.new_block();
        if let Some(exit) = builder.lower_seq(stmts, entry, None) {
            builder.set_term(exit, Terminator::Return);
        }
        ControlFlowGraph {
            blocks: builder.blocks,
            entry,
        }
    }
}

/// Every region's CFG for one file: the top-level plus one per function body,
/// keyed by the `FUNCTION_EXPR`'s [`NodePtr`]. `PartialEq`/`Eq` let salsa
/// backdate the [`control_flow`](crate::incremental::control_flow) query when an
/// edit leaves the graph unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileControlFlow {
    toplevel: ControlFlowGraph,
    functions: Vec<(NodePtr, ControlFlowGraph)>,
}

impl FileControlFlow {
    /// Build every region's CFG from a parsed file root.
    pub fn build(root: &SyntaxNode) -> Self {
        let toplevel = ControlFlowGraph::build_region(&region_statements(root));
        let functions = root
            .descendants()
            .filter_map(FunctionExpr::cast)
            .filter_map(|function| {
                let body = function.body()?;
                let cfg = ControlFlowGraph::build_region(&body_statements(&body));
                Some((NodePtr::from_node(function.syntax()), cfg))
            })
            .collect();
        Self {
            toplevel,
            functions,
        }
    }

    /// The file top-level region's CFG.
    pub fn toplevel(&self) -> &ControlFlowGraph {
        &self.toplevel
    }

    /// Each function body's CFG, in source order, keyed by the `FUNCTION_EXPR`'s
    /// [`NodePtr`].
    pub fn functions(&self) -> &[(NodePtr, ControlFlowGraph)] {
        &self.functions
    }

    /// The CFG for the function pointed at by `ptr`, if present.
    pub fn function(&self, ptr: NodePtr) -> Option<&ControlFlowGraph> {
        self.functions
            .iter()
            .find(|(p, _)| *p == ptr)
            .map(|(_, cfg)| cfg)
    }

    /// Whether the statement at exactly `range` provably cannot be reached: it
    /// lands in an [`Terminator::Unreachable`] block of some region (the tail
    /// after an unconditional divergence — a `return()`/`stop()`, an `if`/`else`
    /// that exits in both arms, or a `repeat` with no `break`). Consulted by the
    /// `unreachable-code` lint.
    pub fn is_unreachable(&self, range: TextRange) -> bool {
        std::iter::once(&self.toplevel)
            .chain(self.functions.iter().map(|(_, cfg)| cfg))
            .flat_map(ControlFlowGraph::blocks)
            .any(|block| {
                matches!(block.terminator, Terminator::Unreachable) && block.stmts.contains(&range)
            })
    }

    /// Render the graph textually (region by region) for snapshot tests. `src`
    /// is the file text the ranges index into.
    pub fn render(&self, src: &str) -> String {
        let mut out = String::new();
        out.push_str("region: <toplevel>\n");
        render_region(&self.toplevel, src, &mut out);
        for (ptr, cfg) in &self.functions {
            let head = snippet(src, ptr.text_range());
            out.push_str(&format!("region: {head}\n"));
            render_region(cfg, src, &mut out);
        }
        out
    }
}

/// Whether executing `element` never falls through to the next statement in its
/// block: it unconditionally `return()`/`stop()`s, recursively through `{}`
/// blocks (any statement diverging is enough) and through `if`/`else` where
/// **both** arms diverge. Purely syntactic — the callee is matched by name, so a
/// caller that must exclude a locally-redefined `return`/`stop` applies its own
/// namespace confirmation on the leaf calls.
///
/// `break`/`next` are deliberately *not* counted here: they leave the loop, not
/// the region, and the block-tail rule that consumes this reasons about
/// `return`/`stop`.
pub fn always_diverges(element: &SyntaxElement) -> bool {
    let Some(node) = element.as_node() else {
        return false;
    };
    match node.kind() {
        SyntaxKind::CALL_EXPR => matches!(
            CallExpr::cast(node.clone())
                .and_then(|call| call.callee_name())
                .as_deref(),
            Some("return" | "stop")
        ),
        SyntaxKind::BLOCK_EXPR => region_statements(node).iter().any(always_diverges),
        SyntaxKind::IF_EXPR => {
            let if_expr = IfExpr::cast(node.clone()).expect("IF_EXPR casts");
            let (Some(then_body), Some(else_body)) = (
                branch_single(if_expr.then_elements()),
                branch_single(if_expr.else_elements()),
            ) else {
                return false;
            };
            always_diverges(&then_body) && always_diverges(&else_body)
        }
        _ => false,
    }
}

/// Builder state: the block arena being filled.
struct Builder {
    blocks: Vec<BasicBlock>,
}

/// The enclosing loop's targets, threaded through lowering so `break`/`next`
/// know where to jump.
#[derive(Clone, Copy)]
struct LoopCtx {
    header: BlockId,
    after: BlockId,
}

impl Builder {
    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            stmts: Vec::new(),
            terminator: Terminator::Return,
        });
        id
    }

    fn set_term(&mut self, block: BlockId, terminator: Terminator) {
        self.blocks[block.0 as usize].terminator = terminator;
    }

    fn push_stmt(&mut self, block: BlockId, range: TextRange) {
        self.blocks[block.0 as usize].stmts.push(range);
    }

    fn has_predecessor(&self, target: BlockId) -> bool {
        self.blocks.iter().any(|block| match block.terminator {
            Terminator::Goto(t) => t == target,
            Terminator::Branch { then_blk, else_blk } => then_blk == target || else_blk == target,
            _ => false,
        })
    }

    /// Lower a statement sequence into `cur`, returning the block control leaves
    /// through, or `None` if control diverges before the sequence ends (the
    /// remaining statements are recorded in an unreachable block).
    fn lower_seq(
        &mut self,
        stmts: &[SyntaxElement],
        mut cur: BlockId,
        loop_ctx: Option<LoopCtx>,
    ) -> Option<BlockId> {
        for (i, stmt) in stmts.iter().enumerate() {
            match self.lower_stmt(stmt, cur, loop_ctx) {
                Some(next) => cur = next,
                None => {
                    let rest = &stmts[i + 1..];
                    if !rest.is_empty() {
                        let dead = self.new_block();
                        self.set_term(dead, Terminator::Unreachable);
                        for stmt in rest {
                            self.push_stmt(dead, stmt.text_range());
                        }
                    }
                    return None;
                }
            }
        }
        Some(cur)
    }

    fn lower_stmt(
        &mut self,
        stmt: &SyntaxElement,
        cur: BlockId,
        loop_ctx: Option<LoopCtx>,
    ) -> Option<BlockId> {
        let Some(node) = stmt.as_node() else {
            // A bare token statement: `break`/`next` (reserved words R lexes as
            // identifiers), or a leaf value like `2`.
            if let Some(token) = stmt.as_token()
                && token.kind() == SyntaxKind::IDENT
            {
                match token.text() {
                    "break" => {
                        return self.lower_jump(cur, stmt.text_range(), loop_ctx, |lc| lc.after);
                    }
                    "next" => {
                        return self.lower_jump(cur, stmt.text_range(), loop_ctx, |lc| lc.header);
                    }
                    _ => {}
                }
            }
            self.push_stmt(cur, stmt.text_range());
            return Some(cur);
        };

        match node.kind() {
            // A bare `{ ... }` inlines into the current flow.
            SyntaxKind::BLOCK_EXPR => self.lower_seq(&region_statements(node), cur, loop_ctx),
            SyntaxKind::IF_EXPR => {
                self.push_stmt(cur, node.text_range());
                self.lower_if(
                    &IfExpr::cast(node.clone()).expect("IF_EXPR casts"),
                    cur,
                    loop_ctx,
                )
            }
            SyntaxKind::FOR_EXPR => {
                self.push_stmt(cur, node.text_range());
                let body = ForExpr::cast(node.clone())
                    .expect("FOR_EXPR casts")
                    .body_element();
                self.lower_loop(cur, body.as_ref(), false)
            }
            SyntaxKind::WHILE_EXPR => {
                self.push_stmt(cur, node.text_range());
                let body = WhileExpr::cast(node.clone())
                    .expect("WHILE_EXPR casts")
                    .body_element();
                self.lower_loop(cur, body.as_ref(), false)
            }
            SyntaxKind::REPEAT_EXPR => {
                self.push_stmt(cur, node.text_range());
                let body = RepeatExpr::cast(node.clone())
                    .expect("REPEAT_EXPR casts")
                    .body();
                self.lower_loop(cur, body.as_ref(), true)
            }
            SyntaxKind::CALL_EXPR => {
                self.push_stmt(cur, node.text_range());
                match CallExpr::cast(node.clone())
                    .and_then(|call| call.callee_name())
                    .as_deref()
                {
                    Some("return" | "stop") => {
                        self.set_term(cur, Terminator::Diverge);
                        None
                    }
                    _ => Some(cur),
                }
            }
            _ => {
                self.push_stmt(cur, node.text_range());
                Some(cur)
            }
        }
    }

    /// Lower a `break`/`next`: record it, then jump to the loop target chosen by
    /// `target`. Outside any loop (invalid R) it is left as a plain statement.
    fn lower_jump(
        &mut self,
        cur: BlockId,
        range: TextRange,
        loop_ctx: Option<LoopCtx>,
        target: impl Fn(LoopCtx) -> BlockId,
    ) -> Option<BlockId> {
        self.push_stmt(cur, range);
        match loop_ctx {
            Some(lc) => {
                self.set_term(cur, Terminator::Goto(target(lc)));
                None
            }
            None => Some(cur),
        }
    }

    fn lower_if(
        &mut self,
        if_expr: &IfExpr,
        cur: BlockId,
        loop_ctx: Option<LoopCtx>,
    ) -> Option<BlockId> {
        let then_blk = self.new_block();
        let then_exit = self.lower_seq(
            &branch_statements(if_expr.then_elements()),
            then_blk,
            loop_ctx,
        );

        if if_expr.else_keyword().is_none() {
            // No `else`: the false path falls straight to the join, so the `if`
            // never fully diverges.
            let join = self.new_block();
            self.set_term(
                cur,
                Terminator::Branch {
                    then_blk,
                    else_blk: join,
                },
            );
            if let Some(exit) = then_exit {
                self.set_term(exit, Terminator::Goto(join));
            }
            return Some(join);
        }

        let else_blk = self.new_block();
        let else_exit = self.lower_seq(
            &branch_statements(if_expr.else_elements()),
            else_blk,
            loop_ctx,
        );
        self.set_term(cur, Terminator::Branch { then_blk, else_blk });

        match (then_exit, else_exit) {
            // Both arms diverge — the code after the `if` is unreachable.
            (None, None) => None,
            _ => {
                let join = self.new_block();
                if let Some(exit) = then_exit {
                    self.set_term(exit, Terminator::Goto(join));
                }
                if let Some(exit) = else_exit {
                    self.set_term(exit, Terminator::Goto(join));
                }
                Some(join)
            }
        }
    }

    /// Lower a `for`/`while`/`repeat`. `for`/`while` may run zero iterations, so
    /// control always reaches the continuation. `repeat` only exits via `break`,
    /// so with no reachable `break` it diverges (the continuation is unreachable).
    fn lower_loop(
        &mut self,
        cur: BlockId,
        body: Option<&SyntaxElement>,
        is_repeat: bool,
    ) -> Option<BlockId> {
        let header = self.new_block();
        self.set_term(cur, Terminator::Goto(header));
        let after = self.new_block();
        let body_blk = self.new_block();
        if is_repeat {
            self.set_term(header, Terminator::Goto(body_blk));
        } else {
            self.set_term(
                header,
                Terminator::Branch {
                    then_blk: body_blk,
                    else_blk: after,
                },
            );
        }

        let loop_ctx = LoopCtx { header, after };
        let body_stmts = body.map(body_statements).unwrap_or_default();
        if let Some(exit) = self.lower_seq(&body_stmts, body_blk, Some(loop_ctx)) {
            self.set_term(exit, Terminator::Goto(header)); // back-edge
        }

        if is_repeat && !self.has_predecessor(after) {
            self.set_term(after, Terminator::Unreachable);
            None
        } else {
            Some(after)
        }
    }
}

/// The statement elements directly inside `container` (a `ROOT` or `BLOCK_EXPR`):
/// its children minus trivia, comments, braces, and `;` separators. Each is a
/// bare token (`x`) or an expression node.
fn region_statements(container: &SyntaxNode) -> Vec<SyntaxElement> {
    container
        .children_with_tokens()
        .filter(|element| {
            !is_trivia(element.kind())
                && !matches!(
                    element.kind(),
                    SyntaxKind::COMMENT
                        | SyntaxKind::LBRACE
                        | SyntaxKind::RBRACE
                        | SyntaxKind::SEMICOLON
                )
        })
        .collect()
}

/// The statements of a body element: a `{ ... }` block expands to its
/// statements; any other expression is a one-statement sequence.
fn body_statements(body: &SyntaxElement) -> Vec<SyntaxElement> {
    match body.as_node() {
        Some(node) if node.kind() == SyntaxKind::BLOCK_EXPR => region_statements(node),
        _ => vec![body.clone()],
    }
}

/// The statements of a single `if` arm (from [`IfExpr::then_elements`] /
/// [`IfExpr::else_elements`]): a `{ ... }` arm expands to its statements, a bare
/// arm is a one-statement sequence.
fn branch_statements(elements: Option<Vec<SyntaxElement>>) -> Vec<SyntaxElement> {
    match branch_single(elements.clone()) {
        Some(single) if single.kind() == SyntaxKind::BLOCK_EXPR => {
            region_statements(single.as_node().expect("BLOCK_EXPR is a node"))
        }
        _ => elements
            .unwrap_or_default()
            .into_iter()
            .filter(|element| !is_trivia(element.kind()) && element.kind() != SyntaxKind::COMMENT)
            .collect(),
    }
}

/// The single non-trivia element of an `if` arm, if the arm is exactly one
/// element (the usual case). `None` for a missing or malformed arm.
fn branch_single(elements: Option<Vec<SyntaxElement>>) -> Option<SyntaxElement> {
    let mut body = elements?
        .into_iter()
        .filter(|element| !is_trivia(element.kind()) && element.kind() != SyntaxKind::COMMENT);
    let first = body.next()?;
    body.next().is_none().then_some(first)
}

fn render_region(cfg: &ControlFlowGraph, src: &str, out: &mut String) {
    for (i, block) in cfg.blocks().iter().enumerate() {
        let stmts = block
            .stmts
            .iter()
            .map(|range| snippet(src, *range))
            .collect::<Vec<_>>()
            .join("; ");
        let term = match block.terminator {
            Terminator::Goto(t) => format!("-> bb{}", t.index()),
            Terminator::Branch { then_blk, else_blk } => {
                format!(
                    "-> then bb{}, else bb{}",
                    then_blk.index(),
                    else_blk.index()
                )
            }
            Terminator::Return => "-> return".to_string(),
            Terminator::Diverge => "-> diverge".to_string(),
            Terminator::Unreachable => "(unreachable)".to_string(),
        };
        out.push_str(&format!("  bb{i}: [{stmts}] {term}\n"));
    }
}

/// A one-line snippet of `src` at `range`, with interior newlines collapsed.
fn snippet(src: &str, range: TextRange) -> String {
    let text = &src[range];
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() > 40 {
        format!(
            "{}…",
            &flat[..flat.char_indices().nth(40).map_or(flat.len(), |(i, _)| i)]
        )
    } else {
        flat
    }
}
