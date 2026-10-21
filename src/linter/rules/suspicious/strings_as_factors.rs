//! Character columns depend on the R 4.0 default change when supported versions
//! span that boundary. Unknown column types stay silent rather than guessing.

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
use crate::config::{CompatConfig, CompatVersion};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct StringsAsFactors;

impl Rule for StringsAsFactors {
    fn id(&self) -> &'static str {
        "strings-as-factors"
    }

    fn description(&self) -> &'static str {
        "Set `stringsAsFactors` explicitly in `data.frame()` when supported R versions \
         span the R 4.0.0 default change from `TRUE` to `FALSE`. The rule runs only \
         with a declared R floor below 4.0, resolved from `[compat] r` or DESCRIPTION. \
         With no floor, or a floor of 4.0 or later, it stays silent.\n\n\
         Known character columns include string literals and base `c()` calls made \
         entirely from atomic literals with at least one string. Unknown values and \
         columns protected by `I()` are left alone. Only base-resolved calls are \
         checked. No fix is offered because either factors or character vectors \
         may be intended."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "With a declared R 3.6 compatibility floor:",
            source: "data.frame(label = c(\"a\", \"b\"))\n",
        }]
    }

    fn doc_compat(&self) -> CompatConfig {
        CompatConfig {
            r: Some("3.6".into()),
            roxygen2: None,
        }
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        if matchers::callee_name(&call).as_deref() != Some("data.frame")
            || !base_call(&call, ctx)
            || !ctx
                .r_compat_floor()
                .is_some_and(|floor| floor < CompatVersion::parse("4.0").unwrap())
        {
            return;
        }
        let args = matchers::args(&call);
        if args
            .iter()
            .any(|arg| arg.name.as_deref() == Some("stringsAsFactors"))
        {
            return;
        }
        let column = args
            .iter()
            .filter(|arg| {
                !matches!(
                    arg.name.as_deref(),
                    Some("row.names" | "check.rows" | "check.names" | "fix.empty.names")
                )
            })
            .filter_map(|arg| arg.value.as_ref())
            .find(|value| known_character(value, ctx));
        let Some(column) = column else {
            return;
        };
        sink.push(Diagnostic {
            rule: self.id(), severity: Default::default(), path: Default::default(),
            range: column.text_range(),
            message: ViolationData::new(self.id(),
                "This character column depends on the `stringsAsFactors` default changed in R 4.0.0")
                .with_suggestion("Set `stringsAsFactors = TRUE` or `stringsAsFactors = FALSE` explicitly."),
            fix: None,
        });
    }
}

fn base_call(call: &CallExpr, ctx: &RuleContext<'_>) -> bool {
    if let Some(access) = call
        .base()
        .and_then(|el| el.into_node())
        .and_then(BinaryExpr::cast)
        .and_then(|expr| expr.namespace_access())
    {
        return access.package == "base";
    }
    ctx.resolves_to_base(call)
        && !ctx.project.is_some_and(|scope| {
            matchers::callee_name(call).is_some_and(|name| scope.resolves(&name))
        })
}

fn known_character(value: &SyntaxElement, ctx: &RuleContext<'_>) -> bool {
    if value.kind() == SyntaxKind::STRING {
        return true;
    }
    let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) else {
        return false;
    };
    if matchers::callee_name(&call).as_deref() != Some("c") || !base_call(&call, ctx) {
        return false;
    }
    let args = matchers::args(&call);
    let mut has_string = false;
    for arg in args {
        let Some(value) = arg.value.and_then(|el| el.into_token()) else {
            return false;
        };
        match value.kind() {
            SyntaxKind::STRING => has_string = true,
            SyntaxKind::INT | SyntaxKind::FLOAT => {}
            SyntaxKind::IDENT
                if matches!(
                    value.text(),
                    "TRUE" | "FALSE" | "NA" | "NA_character_" | "NA_integer_" | "NA_real_" | "NULL"
                ) => {}
            _ => return false,
        }
    }
    has_string
}
