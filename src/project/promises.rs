//! Conservative summaries of package-local R promise behavior.
//!
//! An actual argument is safe for `undefined-symbol` only when its matched
//! formal is proven eager. Capture, forwarding to an unknown callee, an unused
//! formal, and every ambiguous shape remain non-eager—the suppressing direction.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, AssignmentExpr, CallExpr, FunctionExpr, HasArgList as _};
use crate::semantic::{BindingId, BindingKind, ScopeKind, SemanticModel};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub struct PromiseForward {
    pub source: String,
    pub callee: String,
    pub argument_names: Vec<Option<String>>,
    pub argument: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct FunctionPromiseSeed {
    pub formals: Vec<String>,
    pub directly_eager: BTreeSet<String>,
    pub opaque: BTreeSet<String>,
    /// Deduplicated and order-independent so that a second identical
    /// forwarding use, or a reordering of the body, leaves the seed equal and
    /// backdates instead of rebuilding the package aggregate.
    pub forwards: BTreeSet<PromiseForward>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct FunctionPromiseSummary {
    pub formals: Vec<String>,
    pub eager: BTreeSet<String>,
}

impl FunctionPromiseSummary {
    pub fn matched_formal(&self, names: &[Option<SmolStr>], argument: usize) -> Option<String> {
        crate::semantic::match_args_to_formals(names, &self.formals)
            .get(argument)
            .and_then(|formal| formal.map(ToString::to_string))
    }
}

/// The per-package summary map is shared rather than copied: every file in a
/// package resolves against the same one, so [`ExternalResolution`] holding it
/// by value would retain one deep copy per file.
///
/// [`ExternalResolution`]: crate::project::ExternalResolution
pub type PromiseSummaries = Arc<BTreeMap<String, FunctionPromiseSummary>>;

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct PackagePromiseIndex {
    pub by_root: BTreeMap<PathBuf, PromiseSummaries>,
}

pub fn file_promise_seeds(
    model: &SemanticModel,
    root: &SyntaxNode,
) -> BTreeMap<String, FunctionPromiseSeed> {
    let mut out = BTreeMap::new();
    for node in root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::ASSIGNMENT_EXPR)
    {
        let Some(assignment) = AssignmentExpr::cast(node) else {
            continue;
        };
        let Some(name_token) = assignment.target_name_token() else {
            continue;
        };
        let is_file_binding = model.bindings().iter().any(|binding| {
            binding.def_range == name_token.text_range()
                && model.scope(binding.scope).kind == ScopeKind::File
        });
        if !is_file_binding {
            continue;
        }
        let Some(SyntaxElement::Node(value)) = assignment.value_element() else {
            continue;
        };
        let Some(function) = FunctionExpr::cast(value) else {
            continue;
        };
        let seed = seed_function(model, root, &function);
        let name = assignment
            .target_name()
            .expect("a name token has a name")
            .to_string();
        if out.insert(name.clone(), seed).is_some() {
            // Multiple definitions share one namespace slot. Refusing to prove
            // any formal eager is safer than depending on source/load order here.
            out.insert(name, FunctionPromiseSeed::default());
        }
    }
    // A package binding whose function shape we cannot recover is still a
    // possible local callee. Keep an opaque entry so its arguments are
    // suppressed instead of silently falling back to eager evaluation.
    for binding in model.bindings() {
        if matches!(binding.kind, BindingKind::Local | BindingKind::Implicit)
            && model.scope(binding.scope).kind == ScopeKind::File
        {
            out.entry(binding.name.to_string()).or_default();
        }
    }
    out
}

fn seed_function(
    model: &SemanticModel,
    root: &SyntaxNode,
    function: &FunctionExpr,
) -> FunctionPromiseSeed {
    let formals: Vec<String> = function
        .params()
        .into_iter()
        .map(|param| param.name.to_string())
        .collect();
    let Some(scope_id) = model
        .scopes()
        .iter()
        .enumerate()
        .find_map(|(index, scope)| {
            (scope.kind == ScopeKind::Function && scope.range == function.syntax().text_range())
                .then(|| crate::semantic::ScopeId::from_index(index))
        })
    else {
        return FunctionPromiseSeed {
            formals,
            ..FunctionPromiseSeed::default()
        };
    };
    let params: HashMap<BindingId, String> = model
        .scope(scope_id)
        .bindings
        .iter()
        .filter_map(|&id| {
            let binding = model.binding(id);
            (binding.kind == BindingKind::Param).then(|| (id, binding.name.to_string()))
        })
        .collect();

    let mut seed = FunctionPromiseSeed {
        formals,
        ..FunctionPromiseSeed::default()
    };
    let mut used = BTreeSet::new();
    for (index, ident) in model.idents().iter().enumerate() {
        let Some(source) = model
            .ident_bindings(index)
            .iter()
            .find_map(|id| params.get(id))
            .cloned()
        else {
            continue;
        };
        used.insert(source.clone());
        if ident.data_masked {
            seed.opaque.insert(source);
            continue;
        }
        match forwarding_site(root, function, ident.range) {
            ForwardingSite::Direct => {
                seed.directly_eager.insert(source);
            }
            ForwardingSite::Call {
                callee,
                argument_names,
                argument,
            } => {
                seed.forwards.insert(PromiseForward {
                    source,
                    callee,
                    argument_names,
                    argument,
                });
            }
            ForwardingSite::Opaque => {
                seed.opaque.insert(source);
            }
        }
    }
    for formal in &seed.formals {
        if !used.contains(formal) {
            seed.opaque.insert(formal.clone());
        }
    }
    seed
}

enum ForwardingSite {
    Direct,
    Call {
        callee: String,
        argument_names: Vec<Option<String>>,
        argument: usize,
    },
    Opaque,
}

fn forwarding_site(
    root: &SyntaxNode,
    function: &FunctionExpr,
    range: rowan::TextRange,
) -> ForwardingSite {
    // Only a use in the body can force the promise. A formal's default value is
    // itself a promise, forced only when *that* formal is, which this summary
    // does not track.
    let Some(body) = function.body() else {
        return ForwardingSite::Opaque;
    };
    if !body.text_range().contains_range(range) {
        return ForwardingSite::Opaque;
    }
    let element = root.covering_element(range);
    let Some(mut node) = element.into_token().and_then(|token| token.parent()) else {
        return ForwardingSite::Opaque;
    };
    // The innermost enclosing argument position, if any. The whole path up to
    // the body still has to be unconditional, so this is not returned until the
    // walk reaches the function itself.
    let mut site: Option<ForwardingSite> = None;
    loop {
        if node.kind() == SyntaxKind::FUNCTION_EXPR && node != *function.syntax() {
            return ForwardingSite::Opaque;
        }
        // Anything on the path that can skip the use leaves the promise
        // unforced, so no proof of eagerness survives it.
        if bypasses_evaluation(&node) {
            return ForwardingSite::Opaque;
        }
        if node.kind() == SyntaxKind::ARG {
            // A nested argument position forwards through two callees at once
            // (`outer(inner(x))` forces `x` only if `outer` forces its own
            // argument too). The summary tracks a single hop, so stop here.
            if site.is_some() {
                return ForwardingSite::Opaque;
            }
            let Some(arg) = Arg::cast(node.clone()) else {
                return ForwardingSite::Opaque;
            };
            let Some(call_node) = node.parent().and_then(|list| list.parent()) else {
                return ForwardingSite::Opaque;
            };
            let Some(call) = CallExpr::cast(call_node) else {
                return ForwardingSite::Opaque;
            };
            let Some(callee) = call.callee_name() else {
                return ForwardingSite::Opaque;
            };
            let args: Vec<Arg> = call.args().collect();
            let Some(argument) = args
                .iter()
                .position(|candidate| candidate.syntax() == arg.syntax())
            else {
                return ForwardingSite::Opaque;
            };
            site = Some(ForwardingSite::Call {
                callee: callee.to_string(),
                argument_names: args
                    .iter()
                    .map(|arg| arg.name().map(|name| name.to_string()))
                    .collect(),
                argument,
            });
        }
        if node == *function.syntax() {
            return site.unwrap_or(ForwardingSite::Direct);
        }
        let Some(parent) = node.parent() else {
            return ForwardingSite::Opaque;
        };
        node = parent;
    }
}

/// Whether reaching this node still leaves the use it contains skippable. A
/// block needs dominance/CFG proof; a branch, a loop body, and a short-circuit
/// right operand can bypass the use outright.
fn bypasses_evaluation(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::BLOCK_EXPR
        | SyntaxKind::IF_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::REPEAT_EXPR => true,
        SyntaxKind::BINARY_EXPR => node
            .children_with_tokens()
            .any(|element| matches!(element.kind(), SyntaxKind::AND2 | SyntaxKind::OR2)),
        _ => false,
    }
}

pub(crate) fn solve_package(seeds: BTreeMap<String, FunctionPromiseSeed>) -> PromiseSummaries {
    let mut summaries: BTreeMap<String, FunctionPromiseSummary> = seeds
        .iter()
        .map(|(name, seed)| {
            (
                name.clone(),
                FunctionPromiseSummary {
                    formals: seed.formals.clone(),
                    eager: BTreeSet::new(),
                },
            )
        })
        .collect();

    loop {
        let previous = summaries.clone();
        for (name, seed) in &seeds {
            let summary = summaries.get_mut(name).expect("seed installed above");
            for formal in &seed.formals {
                if seed.opaque.contains(formal) {
                    continue;
                }
                let forwards: Vec<_> = seed
                    .forwards
                    .iter()
                    .filter(|forward| &forward.source == formal)
                    .collect();
                let forwards_eager = forwards.iter().all(|forward| {
                    let Some(target) = previous.get(&forward.callee) else {
                        return false;
                    };
                    let names: Vec<Option<SmolStr>> = forward
                        .argument_names
                        .iter()
                        .map(|name| name.as_deref().map(SmolStr::new))
                        .collect();
                    target
                        .matched_formal(&names, forward.argument)
                        .is_some_and(|target_formal| target.eager.contains(&target_formal))
                });
                let has_eager_use = seed.directly_eager.contains(formal) || !forwards.is_empty();
                if has_eager_use && forwards_eager {
                    summary.eager.insert(formal.clone());
                }
            }
        }
        if summaries == previous {
            break;
        }
    }
    Arc::new(summaries)
}
