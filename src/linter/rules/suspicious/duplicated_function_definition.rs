//! `duplicated-function-definition`: the same function name defined twice in
//! one statement list, so the first definition never takes effect.
//!
//! `f <- function(…)` followed later by another `f <- function(…)` among the
//! same run of statements is almost always a copy-paste or merge artifact: R
//! evaluates both, and the second silently replaces the first. Every call goes
//! to the surviving definition, so the earlier body is dead code that still
//! reads as live.
//!
//! The rule is **semantic** (`sem`) on both halves of that claim:
//!
//! - *Same variable.* The definitions are matched through the
//!   [`SemanticModel`]'s bindings, so a nested `helper` inside a closure is a
//!   different variable from a top-level `helper` and the two never pair up.
//! - *First one unused.* A redefinition **after a genuine use** is a deliberate
//!   rewrite, not a duplicate — the first definition did run. The Phase A
//!   def-use index answers this exactly: the pair is reported only when the
//!   earlier binding has no read site before the later definition. Reads inside
//!   the earlier definition's *own* statement are excluded, since a recursive
//!   self-call resolves at call time (to the surviving definition) and so is no
//!   evidence the earlier body was ever reached.
//!
//! Pairing is confined to definitions that are **siblings in one statement
//! list** (both direct children of the same `{ … }` or of the file root). That
//! is what makes "the second overwrites the first" true rather than merely
//! possible: definition-by-condition
//! (`if (x) f <- function() 1 else f <- function() 2`) puts the two in
//! different lists, and only one of them ever runs.
//!
//! There is **no fix** — which of the two definitions the author meant to keep
//! is a judgement call, and both deleting one and renaming it change behavior
//! in ways a mechanical edit cannot choose between.
//!
//! Relation to `unused-binding`: that rule reasons about a binding that is
//! never read *at all*, and a call to the surviving definition marks the whole
//! same-name cohort read — which is exactly why it stays silent on the shape
//! this rule exists to catch.
//!
//! [`SemanticModel`]: crate::semantic::SemanticModel

use rowan::ast::AstNode as _;
use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

use crate::ast::AssignmentExpr;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::{Binding, BindingId, BindingKind};
use crate::syntax::{SyntaxKind, SyntaxNode};
use crate::text::LineIndex;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "The first `calc` is replaced before it is ever called:",
        source: "calc <- function(x) {\n  x + 1\n}\n\ncalc <- function(x) {\n  x * 2\n}\n\ncalc(3)\n",
    },
    Example {
        caption: "The same mistake inside a function body:",
        source: "process <- function(data) {\n  clean <- function(x) x[!is.na(x)]\n  clean <- function(x) x[x > 0]\n  clean(data)\n}\n",
    },
];

pub struct DuplicatedFunctionDefinition;

impl Rule for DuplicatedFunctionDefinition {
    fn id(&self) -> &'static str {
        "duplicated-function-definition"
    }

    fn description(&self) -> &'static str {
        "Flag a function name defined twice among the same run of statements, \
         where the earlier definition is replaced before it is ever used. R \
         evaluates both assignments, so every call reaches the second one and \
         the first body is dead code — nearly always a copy-paste or merge \
         artifact.\n\nOnly definitions that are siblings in one statement list \
         are paired, so definition-by-condition (`if (x) f <- function() 1 \
         else f <- function() 2`) is not flagged: only one branch runs. A \
         redefinition that follows a genuine use of the earlier definition is \
         not flagged either — that is a deliberate rewrite, not a duplicate. \
         No fix is offered: which definition to keep is a judgement call."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let mut sites: Vec<DefSite> = ctx
            .model
            .bindings()
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == BindingKind::Local)
            .filter_map(|(idx, b)| DefSite::of(ctx.root, BindingId::from_index(idx), b))
            .collect();
        sites.sort_by_key(|site| site.def_range.start());

        let mut lines: Option<LineIndex> = None;
        for (i, cur) in sites.iter().enumerate() {
            // The nearest earlier definition of the same name in the same
            // statement list — the one this definition directly overwrites.
            let Some(prev) = sites[..i]
                .iter()
                .rev()
                .find(|prev| prev.list == cur.list && prev.name == cur.name)
            else {
                continue;
            };
            if prev.is_used_before(ctx, cur.def_range.start()) {
                continue;
            }
            let index = lines.get_or_insert_with(|| LineIndex::new(&ctx.root.text().to_string()));
            let line = index.byte_to_lc(usize::from(prev.def_range.start())).line;
            sink.push(Diagnostic {
                rule: "duplicated-function-definition",
                severity: Default::default(),
                path: Default::default(),
                range: cur.def_range,
                message: ViolationData::new(
                    "duplicated-function-definition",
                    format!(
                        "function `{}` is redefined here; the definition on line {line} is \
                         never used",
                        cur.name
                    ),
                )
                .with_suggestion(
                    "Remove the definition that is overwritten, or give one of them a \
                     different name.",
                ),
                fix: None,
            });
        }
    }
}

/// A `name <- function(…)` statement sitting directly in a statement list.
struct DefSite {
    binding: BindingId,
    name: SmolStr,
    /// Range of the defining name token — the finding's span.
    def_range: TextRange,
    /// Range of the whole assignment statement, used to recognize (and discount)
    /// the definition's own recursive self-references.
    statement: TextRange,
    /// Range of the enclosing statement list (`ROOT` or `BLOCK_EXPR`), standing
    /// in for its identity: two definitions share one only when both run, in
    /// order, one after the other.
    list: TextRange,
}

impl DefSite {
    /// The definition site of `binding`, or `None` unless it is the target of a
    /// plain assignment whose value is a function literal and whose statement is
    /// a direct child of a statement list. Every other shape — a chained or
    /// nested assignment, a value that is a call or a constant, a definition
    /// that is itself a branch body — is not a definition this rule pairs.
    fn of(root: &SyntaxNode, id: BindingId, binding: &Binding) -> Option<Self> {
        let token = root.covering_element(binding.def_range).into_token()?;
        let assign = token.parent().and_then(AssignmentExpr::cast)?;
        // The token must be the assignment's *target*, not some other identifier
        // inside it. `target_name_token` also handles right assignment
        // (`function() 1 -> f`), where the target sits after the operator.
        if assign.target_name_token()?.text_range() != binding.def_range {
            return None;
        }
        let value = assign.value_element()?.into_node()?;
        if value.kind() != SyntaxKind::FUNCTION_EXPR {
            return None;
        }
        let list = assign.syntax().parent()?;
        if !matches!(list.kind(), SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR) {
            return None;
        }
        Some(Self {
            binding: id,
            name: binding.name.clone(),
            def_range: binding.def_range,
            statement: assign.syntax().text_range(),
            list: list.text_range(),
        })
    }

    /// Whether this definition is read somewhere before `offset` — the point at
    /// which a later definition replaces it. Reads inside the definition's own
    /// statement don't count: a recursive self-call resolves when the function
    /// runs, by which time the later definition is the one bound.
    fn is_used_before(&self, ctx: &RuleContext<'_>, offset: TextSize) -> bool {
        ctx.model
            .read_sites(self.binding)
            .any(|read| read.range.start() < offset && !self.statement.contains_range(read.range))
    }
}
