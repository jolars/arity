//! S7 language capabilities and the existing CST shapes of named bindings.

use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{AssignmentExpr, AstToken as _, CallExpr, Ident};
use crate::config::CompatVersion;
use crate::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7Version {
    #[default]
    Release,
    Development,
}

impl S7Version {
    pub(crate) fn from_floor(floor: &CompatVersion) -> Self {
        if *floor >= CompatVersion::parse("0.2.2.9000").expect("S7 development floor") {
            Self::Development
        } else {
            Self::Release
        }
    }

    pub(crate) fn development_export(name: &str) -> bool {
        Self::Development
            .additional_exports("S7")
            .iter()
            .any(|export| export == name)
    }

    pub(crate) fn additional_exports(self, package: &str) -> &'static [SmolStr] {
        static DEVELOPMENT_EXPORTS: &[SmolStr] = &[
            SmolStr::new_static(":="),
            SmolStr::new_static("convert_lazy"),
            SmolStr::new_static("deprecated_class"),
            SmolStr::new_static("deprecated_generic"),
            SmolStr::new_static("deprecated_property"),
            SmolStr::new_static("new_external_class"),
            SmolStr::new_static("prop_info"),
            SmolStr::new_static("S4_contains"),
            SmolStr::new_static("S7_class_desc"),
            SmolStr::new_static("S7_classes"),
            SmolStr::new_static("S7_generic_call"),
            SmolStr::new_static("S7_generic_fun"),
            SmolStr::new_static("S7_generics"),
            SmolStr::new_static("S7_methods"),
            SmolStr::new_static("S7_on_load"),
            SmolStr::new_static("S7_on_unload"),
            SmolStr::new_static("S7_on_build"),
            SmolStr::new_static("S7_user_frame"),
        ];
        if self == Self::Development && package == "S7" {
            DEVELOPMENT_EXPORTS
        } else {
            &[]
        }
    }
}

/// Interpret a binding site only after the caller has established operator provenance.
pub(crate) fn binding_parts(node: &SyntaxNode) -> Option<(SyntaxToken, CallExpr)> {
    let (target, value) = if let Some(assignment) = AssignmentExpr::cast(node.clone()) {
        if assignment.op_kind() != Some(SyntaxKind::WALRUS) {
            return None;
        }
        (assignment.target_element()?, assignment.value_element()?)
    } else {
        let call = CallExpr::cast(node.clone())?;
        let base = call.base()?;
        let name = base.as_token().cloned().or_else(|| {
            let binary = crate::ast::BinaryExpr::cast(base.into_node()?)?;
            if !matches!(binary.op_kind()?, SyntaxKind::COLON2 | SyntaxKind::COLON3) {
                return None;
            }
            binary.rhs()?.into_token()
        })?;
        let is_walrus = if name.kind() == SyntaxKind::STRING {
            super::decode_string_literal(name.text()).as_deref() == Some(":=")
        } else {
            super::symbols::unbacktick(name.text()) == ":="
        };
        if !is_walrus {
            return None;
        }
        let list = call.arg_list()?;
        let mut args = list.args();
        let lhs = args.next()?;
        let rhs = args.next()?;
        // S7 takes the operands directly from the call in source order.
        if args.next().is_some() {
            return None;
        }
        (lhs.value()?, rhs.value()?)
    };
    let target = target.into_token()?;
    let ident = Ident::cast(target.clone())?;
    if ident.is_reserved_constant() || ident.is_dots() {
        return None;
    }
    let call = CallExpr::cast(value.into_node()?)?;
    if !call
        .syntax()
        .children_with_tokens()
        .any(|e| e.kind() == SyntaxKind::RPAREN)
        || call.arg_list()?.args().any(|arg| {
            arg.name().is_some_and(|name| name == "name")
                || arg
                    .value()
                    .is_some_and(|value| value.as_token().is_some_and(|t| t.text() == "..."))
        })
    {
        return None;
    }
    Some((target, call))
}
