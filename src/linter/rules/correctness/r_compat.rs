//! `r-compat`: syntax newer than the project's minimum supported R version.
//!
//! A project that declares an R floor — `[compat] r` in `arity.toml`, or
//! `Depends: R (>= …)` in the package `DESCRIPTION` — promises that its code
//! runs there, but nothing checks the promise: `x |> f()` parses fine on the
//! developer's R 4.4 and is a *syntax error* on the declared 4.0. Each
//! construct is flagged against the version that introduced it:
//!
//! - raw string literals (`r"(…)"`) — R 4.0.0;
//! - the native pipe `|>` and the lambda shorthand `\(x)` — R 4.1.0;
//! - the pipe placeholder `_` — R 4.2.0.
//!
//! Without any declared floor the rule stays silent — there is no promise to
//! break, and flagging modern syntax in a loose script would be pure noise.
//!
//! Only the lambda carries a fix: `\(x)` is exact sugar for `function(x)`, so
//! the same-span keyword swap is safe and version-portable. The other
//! constructs have no drop-in textual equivalent (`x |> f()` needs an
//! argument rewrite, a raw string needs re-escaping), so their findings are
//! report-only.

use crate::config::{CompatConfig, CompatVersion};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct RCompat;

const EXAMPLES: &[Example] = &[Example {
    caption: "The native pipe under a declared `R (>= 4.0)` floor \
              (`r = \"4.0\"` under `[compat]` in `arity.toml`):",
    source: "y <- c(1, 2) |> sum()\nprint(y)\n",
}];

/// The construct table: what the matched token is, and the R version that
/// introduced it.
struct Requirement {
    label: &'static str,
    version: &'static str,
}

impl Rule for RCompat {
    fn id(&self) -> &'static str {
        "r-compat"
    }

    fn description(&self) -> &'static str {
        "Flag syntax newer than the project's minimum supported R version.\
         \n\nThe floor comes from `[compat] r` in `arity.toml`, or from \
         `Depends: R (>= …)` in the package `DESCRIPTION` when the key is \
         unset; with neither, the rule stays silent. Raw strings (`r\"(…)\"`) \
         need R 4.0.0, the native pipe `|>` and the lambda shorthand `\\(x)` \
         need 4.1.0, and the pipe placeholder `_` needs 4.2.0 — on an older \
         R, each is a syntax error. Only the lambda carries a fix \
         (`function(x)` is its exact meaning); the pipe and raw strings have \
         no drop-in textual equivalent, so those findings are report-only."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn doc_compat(&self) -> CompatConfig {
        CompatConfig {
            r: Some("4.0".to_string()),
            roxygen2: None,
        }
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[
            SyntaxKind::PIPE,
            SyntaxKind::FUNCTION_KW,
            SyntaxKind::STRING,
            SyntaxKind::IDENT,
        ]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(token) = el.as_token() else {
            return;
        };
        let requirement = match el.kind() {
            SyntaxKind::PIPE => Requirement {
                label: "the native pipe `|>`",
                version: "4.1.0",
            },
            SyntaxKind::FUNCTION_KW if token.text() == "\\" => Requirement {
                label: "the lambda shorthand `\\(…)`",
                version: "4.1.0",
            },
            SyntaxKind::STRING if is_raw_string(token.text()) => Requirement {
                label: "a raw string literal",
                version: "4.0.0",
            },
            // A bare `_` identifier is only valid R as the pipe placeholder,
            // so any occurrence in a clean parse is one (a user's non-syntactic
            // `` `_` `` binding keeps its backticks in the token text).
            SyntaxKind::IDENT if token.text() == "_" => Requirement {
                label: "the pipe placeholder `_`",
                version: "4.2.0",
            },
            _ => return,
        };
        // Resolve the floor only after a candidate construct matched: the
        // DESCRIPTION fallback walks to disk (lazily, once per file).
        let Some(floor) = ctx.r_compat_floor() else {
            return;
        };
        let required = CompatVersion::parse(requirement.version).expect("table versions are valid");
        if floor >= required {
            return;
        }
        let range = token.text_range();
        let fix = (el.kind() == SyntaxKind::FUNCTION_KW).then(|| {
            Fix::safe(
                usize::from(range.start()),
                usize::from(range.end()),
                "function".to_string(),
                "Replace `\\` with `function`".to_string(),
            )
        });
        sink.push(Diagnostic {
            rule: "r-compat",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "r-compat",
                format!(
                    "{} requires R >= {}, but this project supports R >= {floor}",
                    requirement.label, requirement.version
                ),
            )
            .with_suggestion(
                "Raise the floor (`[compat] r` in `arity.toml`, or `Depends: R (>= …)` \
                 in `DESCRIPTION`) or rewrite with older syntax.",
            ),
            fix,
        });
    }
}

/// Whether a `STRING` token is an R raw string literal: `r`/`R` immediately
/// followed by a quote (`r"(…)"`, `R'[…]'`, with optional dashes handled by
/// the lexer — the prefix is what dates it to R 4.0.0).
fn is_raw_string(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('r' | 'R')) && matches!(chars.next(), Some('"' | '\''))
}
