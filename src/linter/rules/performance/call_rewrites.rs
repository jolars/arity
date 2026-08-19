//! Small, namespace-confirmed rewrites of nested or indirect base calls.
//!
//! These rules share the same conservative contract: match an exact argument
//! shape, confirm every semantically relevant name resolves to base R, and
//! preserve each retained argument byte-for-byte. A comment that the rewrite
//! would discard suppresses the fix while leaving the diagnostic visible.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::config::{CompatConfig, CompatVersion};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

fn call(el: &SyntaxElement) -> Option<CallExpr> {
    CallExpr::cast(el.as_node()?.clone())
}

fn values(call: &CallExpr) -> Vec<matchers::ArgMatch> {
    matchers::args(call)
        .into_iter()
        .filter(|a| a.value.is_some())
        .collect()
}

fn bare_base(el: &SyntaxElement, name: &str, ctx: &RuleContext<'_>) -> bool {
    el.as_token().is_some_and(|t| {
        t.kind() == SyntaxKind::IDENT && t.text() == name && ctx.read_resolves_to_base(t)
    })
}

fn has_dropped_comment(call: &CallExpr, kept: &[rowan::TextRange]) -> bool {
    call.syntax().descendants_with_tokens().any(|e| {
        e.kind() == SyntaxKind::COMMENT && !kept.iter().any(|r| r.contains_range(e.text_range()))
    })
}

fn emit(
    rule: &'static str,
    call: &CallExpr,
    replacement: String,
    suggestion: &'static str,
    kept: &[rowan::TextRange],
    safe: bool,
    sink: &mut Vec<Diagnostic>,
) {
    let range = call.syntax().text_range();
    let fix = (!has_dropped_comment(call, kept)).then(|| {
        let start = range.start().into();
        let end = range.end().into();
        let title = format!("Replace with `{suggestion}`");
        if safe {
            Fix::safe(start, end, replacement, title)
        } else {
            Fix::unsafe_(start, end, replacement, title)
        }
    });
    sink.push(Diagnostic {
        rule,
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(
            rule,
            format!("Use `{suggestion}` instead of this indirect base call."),
        )
        .with_suggestion(format!("Use `{suggestion}`.")),
        fix,
    });
}

macro_rules! rule_meta {
    ($ty:ident, $id:literal, $description:literal, $caption:literal, $source:literal) => {
        pub struct $ty;
        impl Rule for $ty {
            fn id(&self) -> &'static str {
                $id
            }
            fn description(&self) -> &'static str {
                $description
            }
            fn examples(&self) -> &'static [Example] {
                const EXAMPLES: &[Example] = &[Example {
                    caption: $caption,
                    source: $source,
                }];
                EXAMPLES
            }
            fn interests(&self) -> &'static [SyntaxKind] {
                &[SyntaxKind::CALL_EXPR]
            }
            fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
                self.check_call(el, ctx, sink)
            }
        }
    };
}

rule_meta!(
    MatrixApply,
    "matrix-apply",
    "Flag `apply(x, 1/2, sum/mean)` when the corresponding `rowSums`, `colSums`, `rowMeans`, or `colMeans` call is clearer and faster. The exact supported argument shape and all relevant base names are verified before rewriting.",
    "Dedicated matrix row and column helpers:",
    "totals <- apply(x, 1, sum)\n"
);
impl MatrixApply {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = call(el).filter(|c| matchers::callee_name(c).as_deref() == Some("apply"))
        else {
            return;
        };
        let a = values(&call);
        if !(a.len() == 3 || a.len() == 4) || a[0..3].iter().any(|x| x.name.is_some()) {
            return;
        }
        if a.len() == 4 && a[3].name.as_deref() != Some("na.rm") {
            return;
        }
        let (Some(x), Some(margin), Some(fun)) = (
            a[0].value.as_ref(),
            a[1].value.as_ref(),
            a[2].value.as_ref(),
        ) else {
            return;
        };
        let margin = margin
            .as_token()
            .map(|t| t.text())
            .filter(|x| matches!(*x, "1" | "1L" | "2" | "2L"));
        let Some(margin) = margin else { return };
        let fun_name = if bare_base(fun, "sum", ctx) {
            "Sums"
        } else if bare_base(fun, "mean", ctx) {
            "Means"
        } else {
            return;
        };
        if !ctx.resolves_to_base(&call) {
            return;
        }
        let prefix = if margin.starts_with('1') {
            "row"
        } else {
            "col"
        };
        let mut replacement = format!("{prefix}{fun_name}({}", matchers::element_text(x));
        let mut kept = vec![x.text_range()];
        if let Some(extra) = a.get(3).and_then(|x| x.value.as_ref()) {
            replacement.push_str(&format!(", na.rm = {}", matchers::element_text(extra)));
            kept.push(extra.text_range());
        }
        replacement.push(')');
        let label = if prefix == "row" {
            if fun_name == "Sums" {
                "rowSums(x)"
            } else {
                "rowMeans(x)"
            }
        } else if fun_name == "Sums" {
            "colSums(x)"
        } else {
            "colMeans(x)"
        };
        emit(
            "matrix-apply",
            &call,
            replacement,
            label,
            &kept,
            false,
            sink,
        );
    }
}

rule_meta!(
    WhichGrepl,
    "which-grepl",
    "Flag `which(grepl(pattern, x))`, which makes two passes where `grep(pattern, x)` directly returns matching indices. Both calls must resolve to base R.",
    "Direct matching indices:",
    "i <- which(grepl(\"^a\", x))\n"
);
impl WhichGrepl {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(outer) = call(el).filter(|c| matchers::callee_name(c).as_deref() == Some("which"))
        else {
            return;
        };
        let Some(inner_el) = matchers::sole_positional(&outer) else {
            return;
        };
        let Some(inner) = inner_el
            .as_node()
            .and_then(|n| matchers::call_named(n, "grepl"))
        else {
            return;
        };
        if !ctx.resolves_to_base(&outer) || !ctx.resolves_to_base(&inner) {
            return;
        }
        let args = inner.syntax().text().to_string();
        let replacement = format!("grep{}", &args["grepl".len()..]);
        emit(
            "which-grepl",
            &outer,
            replacement,
            "grep(pattern, x)",
            &[inner.syntax().text_range()],
            true,
            sink,
        );
    }
}

rule_meta!(
    RepLen,
    "rep-len",
    "Flag the exact `rep(x, length.out = n)` shape, for which `rep_len(x, n)` is the direct base primitive. Calls with `times`, `each`, or other arguments are excluded.",
    "Direct length-limited repetition:",
    "y <- rep(x, length.out = n)\n"
);
impl RepLen {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = call(el).filter(|c| matchers::callee_name(c).as_deref() == Some("rep"))
        else {
            return;
        };
        let a = values(&call);
        if a.len() != 2
            || a[0].name.is_some()
            || a[1].name.as_deref() != Some("length.out")
            || !ctx.resolves_to_base(&call)
        {
            return;
        }
        let (x, n) = (a[0].value.as_ref().unwrap(), a[1].value.as_ref().unwrap());
        emit(
            "rep-len",
            &call,
            format!(
                "rep_len({}, {})",
                matchers::element_text(x),
                matchers::element_text(n)
            ),
            "rep_len(x, n)",
            &[x.text_range(), n.text_range()],
            true,
            sink,
        );
    }
}

rule_meta!(
    SystemFile,
    "system-file",
    "Flag redundant nesting of base `file.path()` and `system.file()`. `system.file()` already accepts path components through `...`, so exact clean shapes can be flattened safely.",
    "Path components passed directly to `system.file`:",
    "p <- system.file(file.path(\"a\", \"b\"), package = \"pkg\")\n"
);
impl SystemFile {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(outer) = call(el) else { return };
        let name = matchers::callee_name(&outer);
        let a = values(&outer);
        let (inner, mut components, package) = if name.as_deref() == Some("system.file") {
            if a.len() != 2 || a[0].name.is_some() || a[1].name.as_deref() != Some("package") {
                return;
            }
            let Some(inner) = a[0]
                .value
                .as_ref()
                .and_then(|e| e.as_node())
                .and_then(|n| matchers::call_named(n, "file.path"))
            else {
                return;
            };
            let components = values(&inner)
                .into_iter()
                .filter_map(|x| x.value)
                .collect::<Vec<_>>();
            (inner, components, a[1].value.clone().unwrap())
        } else if name.as_deref() == Some("file.path") {
            if a.len() < 2 || a.iter().any(|x| x.name.is_some()) {
                return;
            }
            let Some(inner) = a[0]
                .value
                .as_ref()
                .and_then(|e| e.as_node())
                .and_then(|n| matchers::call_named(n, "system.file"))
            else {
                return;
            };
            let ia = values(&inner);
            if ia.len() != 1 || ia[0].name.as_deref() != Some("package") {
                return;
            }
            (
                inner,
                a.into_iter().skip(1).filter_map(|x| x.value).collect(),
                ia[0].value.clone().unwrap(),
            )
        } else {
            return;
        };
        if components.is_empty() || !ctx.resolves_to_base(&outer) || !ctx.resolves_to_base(&inner) {
            return;
        }
        let mut kept: Vec<_> = components.iter().map(SyntaxElement::text_range).collect();
        kept.push(package.text_range());
        let body = components
            .drain(..)
            .map(|x| matchers::element_text(&x))
            .collect::<Vec<_>>()
            .join(", ");
        emit(
            "system-file",
            &outer,
            format!(
                "system.file({body}, package = {})",
                matchers::element_text(&package)
            ),
            "system.file(..., package = pkg)",
            &kept,
            true,
            sink,
        );
    }
}

pub struct List2df;
impl Rule for List2df {
    fn id(&self) -> &'static str {
        "list2df"
    }
    fn description(&self) -> &'static str {
        "Flag `do.call(cbind.data.frame, x)` in favor of `list2DF(x)` on R 4.0 or newer; exact arguments and base resolution avoid changing recycling or dispatch behavior."
    }
    fn examples(&self) -> &'static [Example] {
        const E: &[Example] = &[Example {
            caption: "A list converted directly to a data frame:",
            source: "df <- do.call(cbind.data.frame, x)\n",
        }];
        E
    }
    fn doc_compat(&self) -> CompatConfig {
        CompatConfig {
            r: Some("4.0".into()),
            roxygen2: None,
        }
    }
    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }
    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        self.check_call(el, ctx, sink)
    }
}
impl List2df {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) =
            call(el).filter(|c| matchers::callee_name(c).as_deref() == Some("do.call"))
        else {
            return;
        };
        if ctx
            .r_compat_floor()
            .is_some_and(|v| v < CompatVersion::parse("4.0").unwrap())
        {
            return;
        }
        let a = values(&call);
        if a.len() != 2 || a.iter().any(|x| x.name.is_some()) || !ctx.resolves_to_base(&call) {
            return;
        }
        let fun = a[0].value.as_ref().unwrap();
        if !bare_base(fun, "cbind.data.frame", ctx) {
            return;
        }
        let x = a[1].value.as_ref().unwrap();
        emit(
            "list2df",
            &call,
            format!("list2DF({})", matchers::element_text(x)),
            "list2DF(x)",
            &[x.text_range()],
            false,
            sink,
        );
    }
}

rule_meta!(
    LengthLevels,
    "length-levels",
    "Flag `length(levels(x))`, for which base `nlevels(x)` expresses the intent directly. Both nested calls must resolve to base R.",
    "A factor's number of levels:",
    "n <- length(levels(x))\n"
);
impl LengthLevels {
    fn check_call(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(outer) =
            call(el).filter(|c| matchers::callee_name(c).as_deref() == Some("length"))
        else {
            return;
        };
        let Some(inner_el) = matchers::sole_positional(&outer) else {
            return;
        };
        let Some(inner) = inner_el
            .as_node()
            .and_then(|n| matchers::call_named(n, "levels"))
        else {
            return;
        };
        let Some(x) = matchers::sole_positional(&inner) else {
            return;
        };
        if !ctx.resolves_to_base(&outer) || !ctx.resolves_to_base(&inner) {
            return;
        }
        emit(
            "length-levels",
            &outer,
            format!("nlevels({})", matchers::element_text(&x)),
            "nlevels(x)",
            &[x.text_range()],
            true,
            sink,
        );
    }
}
