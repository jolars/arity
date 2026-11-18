//! Project provenance for the development S7 naming operator.

use std::collections::HashSet;

use crate::namespace::DirectiveKind;
use crate::rindex::provider::{IndexedProvider, package_indexed, resolve_origin_with_s7};
use crate::rindex::remote::RemoteExports;
use crate::semantic::s7::binding_parts;
use crate::semantic::{LoadedPackage, PackageOrigin};

use super::*;

pub(crate) struct S7BindingContext<'a> {
    pub namespace: Option<&'a str>,
    pub attached: &'a [String],
    pub shadowed: &'a BTreeSet<String>,
    pub opaque: bool,
    pub indexed: &'a IndexedProvider,
    pub remote: &'a RemoteExports,
}

impl S7BindingContext<'_> {
    fn exports(&self, package: &str, name: &str) -> Option<bool> {
        if !package_indexed(self.indexed, self.remote, package) {
            return None;
        }
        let attached = [LoadedPackage {
            name: package.into(),
            range: TextRange::default(),
        }];
        Some(
            match resolve_origin_with_s7(
                self.indexed,
                self.remote,
                name,
                &attached,
                S7Version::Development,
            ) {
                PackageOrigin::Resolved(origin) => origin == package,
                PackageOrigin::Ambiguous(origins) => origins.iter().any(|origin| origin == package),
                PackageOrigin::Unknown => false,
            },
        )
    }

    // An unknown import can mask an earlier one. A subsequent explicit import
    // still supplies the name in R's namespace environment.
    fn imported_origin(
        &self,
        name: &str,
        document: Option<&crate::namespace::Document>,
    ) -> Option<Option<FunctionIdentity>> {
        let mut origin = None;
        for directive in document
            .into_iter()
            .flat_map(|document| document.directives())
        {
            let imported = match directive.kind() {
                DirectiveKind::ImportFrom => {
                    let mut args = directive.arguments();
                    let Some(package) = args
                        .next()
                        .and_then(|arg| resolve::namespace_name(arg.value()?))
                    else {
                        origin = Some(None);
                        continue;
                    };
                    args.find_map(|arg| {
                        let source = resolve::namespace_name(arg.value()?)?;
                        let local = arg
                            .name()
                            .map_or_else(|| source.clone(), |name| name.to_string());
                        (unbacktick(&local) == name).then(|| {
                            Some(FunctionIdentity {
                                package: package.clone(),
                                name: source,
                            })
                        })
                    })
                }
                DirectiveKind::Import => {
                    let Some((packages, exclusions)) =
                        resolve::namespace::wildcard_import(&directive)
                    else {
                        origin = Some(None);
                        continue;
                    };
                    if exclusions.contains(name) {
                        continue;
                    }
                    packages
                        .into_iter()
                        .filter_map(|package| match self.exports(&package, name) {
                            Some(false) => None,
                            Some(true) => Some(Some(FunctionIdentity {
                                package,
                                name: name.into(),
                            })),
                            None => Some(None),
                        })
                        .next_back()
                }
                _ => None,
            };
            if let Some(imported) = imported {
                let conditional = directive
                    .syntax()
                    .ancestors()
                    .any(|node| node.kind() == SyntaxKind::IF_EXPR);
                origin = Some(if conditional { None } else { imported });
            }
        }
        origin
    }

    fn attached_origin(&self, name: &str, attached: &[String]) -> Option<FunctionIdentity> {
        let search: Vec<_> = self
            .attached
            .iter()
            .chain(attached)
            .map(String::as_str)
            .collect();
        let s7_attached = name == ":=" && search.contains(&"S7");
        search
            .into_iter()
            .rev()
            .find_map(|package| {
                // S7's attach hook replaces or excludes the two NSE sentinels.
                if s7_attached && matches!(package, "rlang" | "data.table") {
                    return None;
                }
                match self.exports(package, name) {
                    Some(false) => None,
                    Some(true) => Some(Some(FunctionIdentity {
                        package: package.into(),
                        name: name.into(),
                    })),
                    None => Some(None),
                }
            })
            .flatten()
    }

    fn function_origin(
        &self,
        reference: FunctionRef,
        document: Option<&crate::namespace::Document>,
    ) -> Option<FunctionIdentity> {
        match reference {
            FunctionRef::Qualified { package, name } => Some(FunctionIdentity { package, name }),
            FunctionRef::Bare {
                name,
                attached,
                attachment_uncertain,
            } if !self.opaque && !self.shadowed.contains(&name) => {
                self.imported_origin(&name, document).unwrap_or_else(|| {
                    (!attachment_uncertain)
                        .then(|| self.attached_origin(&name, &attached))
                        .flatten()
                })
            }
            _ => None,
        }
    }

    fn dynamic_dots(
        &self,
        model: &SemanticModel,
        root: &SyntaxNode,
        node: &SyntaxNode,
        document: Option<&crate::namespace::Document>,
    ) -> bool {
        let infix = node.kind() == SyntaxKind::ASSIGNMENT_EXPR;
        let explicit = CallExpr::cast(node.clone())
            .and_then(|call| call.base())
            .and_then(resolve::namespace_name)
            .as_deref()
            == Some(":=");
        if !infix && !explicit {
            return false;
        }
        let Some(arg) = node.parent().and_then(crate::ast::Arg::cast) else {
            return false;
        };
        let Some(call) = arg
            .syntax()
            .parent()
            .and_then(|args| args.parent())
            .and_then(CallExpr::cast)
        else {
            return false;
        };
        let Some(origin) = call_function_ref(model, root, &call)
            .and_then(|reference| self.function_origin(reference, document))
        else {
            return false;
        };
        let formals: &[&str] = match (origin.package.as_str(), origin.name.as_str()) {
            ("tibble", "tibble") => &["...", ".rows", ".name_repair"],
            ("rlang", "call2") => &[".fn", "...", ".ns"],
            ("rlang", "exec") => &[".fn", "...", ".env"],
            ("rlang", "list2" | "pairlist2") => &["..."],
            ("rlang", "dots_list") => &[
                "...",
                ".named",
                ".ignore_empty",
                ".preserve_empty",
                ".homonyms",
                ".check_assign",
            ],
            ("rlang", "exprs" | "quos") => &["...", ".named", ".ignore_empty", ".unquote_names"],
            ("rlang", "enquos" | "ensyms") => &[
                "...",
                ".named",
                ".ignore_empty",
                ".ignore_null",
                ".unquote_names",
                ".homonyms",
                ".check_assign",
            ],
            _ => return false,
        };
        let Some(list) = call.arg_list() else {
            return false;
        };
        let names: Vec<_> = list.args().map(|arg| arg.name()).collect();
        let Some(index) = list
            .args()
            .position(|candidate| candidate.syntax() == arg.syntax())
        else {
            return false;
        };
        crate::semantic::checked_match_args_to_formals(&names, formals)
            .is_ok_and(|matched| matched[index].is_none())
    }
}

pub(crate) fn s7_binding_sites(
    model: &SemanticModel,
    root: &SyntaxNode,
    context: S7BindingContext<'_>,
) -> HashSet<TextRange> {
    let namespace = context.namespace.map(crate::namespace::parse);
    let invalid_namespace = namespace
        .as_ref()
        .is_some_and(|namespace| !namespace.diagnostics.is_empty());
    let document = namespace.as_ref().map(|namespace| namespace.document());
    root.descendants()
        .filter_map(|node| {
            let (target, _) = binding_parts(&node)?;
            if context.dynamic_dots(model, root, &node, document.as_ref()) {
                return None;
            }
            let reference = if let Some(call) = CallExpr::cast(node.clone()) {
                if let Some(token) = call.base().and_then(SyntaxElement::into_token) {
                    let name = resolve::namespace_name(SyntaxElement::Token(token.clone()))?;
                    operator_reference(model, root, &token, &name)?
                } else {
                    call_function_ref(model, root, &call)?
                }
            } else {
                operator_reference(model, root, &target, ":=")?
            };
            if invalid_namespace && !matches!(reference, FunctionRef::Qualified { .. }) {
                return None;
            }
            let origin = context.function_origin(reference, document.as_ref())?;
            (origin.package == "S7" && origin.name == ":=").then(|| node.text_range())
        })
        .collect()
}

fn operator_reference(
    model: &SemanticModel,
    root: &SyntaxNode,
    token: &SyntaxToken,
    name: &str,
) -> Option<FunctionRef> {
    let scope = model.innermost_scope_at(token.text_range().start());
    let mut frame = Some(scope);
    while let Some(id) = frame {
        let bindings = model.scope_bindings_named(id, &name.into());
        let mut candidates = bindings.iter().copied().filter(|&binding| {
            id != scope || model.binding(binding).available_from <= token.text_range().start()
        });
        if let Some(binding) = candidates.next() {
            if candidates.next().is_some() {
                return None;
            }
            return stable_value(model, root, binding).and_then(|value| {
                function_ref(model, root, &value, &mut BTreeSet::from([binding.0]))
            });
        }
        frame = model.scope(id).parent;
    }
    free_function_ref(model, root, token, name)
}

/// Whether a library/require attachment exposes the naming operator.
pub(super) fn attachment_exposes_operator(
    model: &SemanticModel,
    call: &CallExpr,
    function: &str,
) -> Option<bool> {
    let args = matched_args(call, attachment_formals(function)?)?;
    match (args.get("include.only"), args.get("exclude")) {
        (None, None) => Some(true),
        (Some(include), None) => filter_contains_operator(model, include.as_ref()?),
        (None, Some(exclude)) => Some(!filter_contains_operator(model, exclude.as_ref()?)?),
        (Some(_), Some(_)) => None,
    }
}

fn filter_contains_operator(model: &SemanticModel, value: &SyntaxElement) -> Option<bool> {
    if constant_atom(value, "NULL") {
        return Some(false);
    }
    if let Some(token) = value.as_token() {
        return crate::semantic::decode_string_literal(token.text()).map(|name| name == ":=");
    }
    let node = value.as_node()?.clone();
    if let Some(paren) = crate::ast::ParenExpr::cast(node.clone()) {
        return filter_contains_operator(model, &paren.inner()?);
    }
    let call = CallExpr::cast(node)?;
    let base = call.base()?;
    let factory = match base {
        SyntaxElement::Token(token) if read_binding(model, &token).is_none() => {
            unbacktick(token.text()).to_owned()
        }
        SyntaxElement::Node(node) => {
            let access = BinaryExpr::cast(node)?.namespace_access()?;
            if access.package != "base" {
                return None;
            }
            access.name.to_string()
        }
        _ => return None,
    };
    let list = call.arg_list()?;
    if factory == "character" && list.args().next().is_none() {
        return Some(false);
    }
    if factory != "c" {
        return None;
    }
    let mut includes = false;
    for arg in list.args() {
        if arg.name().is_some() {
            return None;
        }
        includes |= filter_contains_operator(model, &arg.value()?)?;
    }
    Some(includes)
}
