//! Callee provenance and R argument matching shared by OOP declarations.

use smol_str::SmolStr;

use super::*;

pub(super) fn expected_package(name: &str) -> Option<&'static str> {
    // Bare spellings establish candidates only. ClassIndex confirms their
    // origins using package imports, workspace masks, and the search path.
    Some(match name {
        "R6Class" => "R6",
        "setClass" | "setRefClass" | "setGeneric" | "setMethod" | "setReplaceMethod"
        | "signature" | "new" | "getRefClass" | "setClassUnion" | "setOldClass" | "setIs"
        | "setAs" | "representation" | "prototype" | "slot" | "slot<-" | "as" | "as<-"
        | "callNextMethod" | "selectMethod" | "removeMethod" | "setValidity" => "methods",
        "new_class"
        | "new_generic"
        | "method<-"
        | "method"
        | "new_property"
        | "new_object"
        | "prop"
        | "prop<-"
        | "props"
        | "props<-"
        | "set_props"
        | "new_union"
        | "new_S3_class"
        | "new_external_generic"
        | "new_external_class"
        | "S7_dispatch"
        | "super"
        | "S4_register"
        | "S4_contains"
        | "methods_register"
        | "S7_on_load"
        | "S7_on_unload"
        | "S7_on_build" => "S7",
        "getClass" | "getClassDef" | "isClass" | "extends" | "is" => "methods",
        name if super::s7::is_builtin(name) => "S7",
        "UseMethod" | "NextMethod" | ".S3method" | "registerS3method" | "c" | "list"
        | "character" | "structure" | "class" | "class<-" | "oldClass" | "oldClass<-"
        | "unclass" | "return" | "lockBinding" | "unlockBinding" | "do.call" | "environment"
        | "globalenv" | "@" | "@<-" | "standardGeneric" | "library" | "require"
        | "requireNamespace" | "loadNamespace" | "unloadNamespace" | "|" => "base",
        _ => return None,
    })
}

pub(super) fn possible_functions(reference: &FunctionRef) -> Vec<FunctionIdentity> {
    match reference {
        FunctionRef::Qualified { package, name } => vec![FunctionIdentity {
            package: package.clone(),
            name: name.clone(),
        }],
        FunctionRef::Bare { name, .. } => expected_package(name)
            .map(|package| FunctionIdentity {
                package: package.into(),
                name: name.clone(),
            })
            .into_iter()
            .collect(),
        FunctionRef::Unresolved(candidates) => candidates.clone(),
    }
}

pub(super) fn replacement_assignment(call: &CallExpr) -> Option<AssignmentExpr> {
    call.syntax()
        .parent()
        .and_then(AssignmentExpr::cast)
        .filter(|assignment| {
            assignment
                .target_element()
                .is_some_and(|target| target.text_range() == call.syntax().text_range())
        })
}

pub(super) fn call_function_ref(
    model: &SemanticModel,
    root: &SyntaxNode,
    call: &CallExpr,
) -> Option<FunctionRef> {
    let base = call.base()?;
    if replacement_assignment(call).is_none() {
        return function_ref(model, root, &base, &mut BTreeSet::new());
    }
    // R looks up the setter independently. An alias of the getter does not
    // create an alias of its replacement function.
    if let Some(node) = base.as_node() {
        let namespace = BinaryExpr::cast(node.clone())?.namespace_access()?;
        return Some(FunctionRef::Qualified {
            package: namespace.package.to_string(),
            name: format!("{}<-", namespace.name),
        });
    }
    let token = base.as_token()?;
    let name = format!("{}<-", crate::ast::token_name(token));
    named_function_ref(model, root, token, &name, &mut BTreeSet::new())
}

pub(super) fn factories(
    model: &SemanticModel,
    root: &SyntaxNode,
) -> Vec<(CallExpr, FunctionRef, String)> {
    root.descendants()
        .filter_map(CallExpr::cast)
        .flat_map(|call| {
            let Some(reference) = call_function_ref(model, root, &call) else {
                return Vec::new();
            };
            possible_functions(&reference)
                .into_iter()
                .filter(|identity| {
                    expected_package(&identity.name) == Some(identity.package.as_str())
                })
                .map(|identity| (call.clone(), reference.clone(), identity.name))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(crate) fn class_lookups(
    model: &SemanticModel,
    root: &SyntaxNode,
    body_local: bool,
) -> Vec<FunctionRef> {
    factories(model, root)
        .into_iter()
        .filter(|(call, _, name)| {
            let selection_helper = matches!(name.as_str(), "signature" | "c" | "character")
                && call
                    .syntax()
                    .ancestors()
                    .skip(1)
                    .find_map(CallExpr::cast)
                    .and_then(|parent| call_function_ref(model, root, &parent))
                    .is_some_and(|reference| {
                        possible_functions(&reference).iter().any(|identity| {
                            identity.package == "methods" && identity.name == "selectMethod"
                        })
                    });
            (matches!(
                name.as_str(),
                "getRefClass"
                    | "getClass"
                    | "getClassDef"
                    | "isClass"
                    | "extends"
                    | "is"
                    | "setIs"
                    | "setAs"
                    | "setValidity"
                    | "new_external_class"
                    | "new"
                    | "slot"
                    | "slot<-"
                    | "as"
                    | "as<-"
                    | "@"
                    | "@<-"
                    | "standardGeneric"
                    | "callNextMethod"
                    | "selectMethod"
                    | "library"
                    | "require"
                    | "requireNamespace"
                    | "loadNamespace"
                    | "unloadNamespace"
                    | "prop"
                    | "prop<-"
                    | "props"
                    | "props<-"
                    | "set_props"
                    | "new_object"
                    | "S7_dispatch"
                    | "super"
                    | "method"
            ) || selection_helper)
                && call
                    .syntax()
                    .ancestors()
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                    == body_local
        })
        .map(|(_, reference, _)| reference)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

type SuppliedArgument = (Option<SmolStr>, Option<SyntaxElement>);
pub(super) type MatchedArguments = BTreeMap<String, Option<SyntaxElement>>;

fn supplied_args(call: &CallExpr) -> Option<Vec<SuppliedArgument>> {
    if !call
        .syntax()
        .children_with_tokens()
        .any(|e| e.kind() == SyntaxKind::RPAREN)
    {
        return None;
    }
    call.arg_list()?
        .args()
        .map(|arg| {
            let value = arg.value();
            if value
                .as_ref()
                .is_some_and(|value| matches!(value, SyntaxElement::Token(t) if t.text() == "..."))
            {
                return None;
            }
            Some((arg.name(), value))
        })
        .collect()
}

fn match_supplied(
    supplied: Vec<SuppliedArgument>,
    formals: &[&str],
) -> Option<(MatchedArguments, Vec<SuppliedArgument>)> {
    let names: Vec<_> = supplied.iter().map(|(name, _)| name.clone()).collect();
    let matched = crate::semantic::checked_match_args_to_formals(&names, formals).ok()?;
    let mut result = BTreeMap::new();
    let mut dots = Vec::new();
    for ((name, value), formal) in supplied.into_iter().zip(matched) {
        if let Some(formal) = formal {
            result.insert(formal.to_string(), value);
        } else {
            dots.push((name, value));
        }
    }
    Some((result, dots))
}

pub(super) fn attachment_formals(name: &str) -> Option<&'static [&'static str]> {
    let formals: &[_] = match name {
        "library" => &[
            "package",
            "help",
            "pos",
            "lib.loc",
            "character.only",
            "logical.return",
            "warn.conflicts",
            "quietly",
            "verbose",
            "mask.ok",
            "exclude",
            "include.only",
            "attach.required",
        ],
        "require" => &[
            "package",
            "lib.loc",
            "quietly",
            "warn.conflicts",
            "character.only",
            "mask.ok",
            "exclude",
            "include.only",
            "attach.required",
        ],
        _ => return None,
    };
    Some(formals)
}

pub(super) fn matched_args(call: &CallExpr, formals: &[&str]) -> Option<MatchedArguments> {
    Some(match_supplied(supplied_args(call)?, formals)?.0)
}

pub(super) fn declaration_args(
    model: &SemanticModel,
    call: &CallExpr,
    formals: &[&str],
) -> Option<MatchedArguments> {
    let mut supplied = supplied_args(call)?;
    if s7_named_target(model, call.syntax()).is_some() {
        supplied.push((Some("name".into()), None));
    }
    Some(match_supplied(supplied, formals)?.0)
}

pub(super) fn argument(args: &MatchedArguments, name: &str) -> Option<SyntaxElement> {
    args.get(name)?.clone()
}

pub(super) fn constant_atom(value: &SyntaxElement, text: &str) -> bool {
    match value {
        SyntaxElement::Token(token) => token.text() == text,
        SyntaxElement::Node(node) => crate::ast::ParenExpr::cast(node.clone())
            .and_then(|paren| paren.inner())
            .is_some_and(|inner| constant_atom(&inner, text)),
    }
}

pub(super) fn registration_args(
    call: &CallExpr,
    name: &str,
    formals: &[&str],
) -> Option<MatchedArguments> {
    let mut supplied = supplied_args(call)?;
    if matches!(name, "method<-" | "prop<-" | "props<-")
        && let Some(assignment) = replacement_assignment(call)
    {
        supplied.push((Some("value".into()), assignment.value_element()));
    }
    if name == "setReplaceMethod" {
        // The wrapper consumes only exact `where`, then forwards dots to
        // setMethod with `f` and `where` already supplied by name.
        let (mut outer, dots) = match_supplied(supplied, &["f", "...", "where"])?;
        supplied = vec![(Some("f".into()), outer.remove("f").flatten())];
        supplied.extend(dots);
        supplied.push((Some("where".into()), outer.remove("where").flatten()));
    }
    Some(match_supplied(supplied, formals)?.0)
}
