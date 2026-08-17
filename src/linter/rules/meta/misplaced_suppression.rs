//! `misplaced-suppression`: a directive that sits where it can never take
//! effect.
//!
//! Two shapes, both silent by construction — a directive that does nothing
//! looks exactly like one that worked:
//!
//! - A **format** directive outside a statement list. The formatter splices
//!   skipped source back line by line, which it can only do where the lines
//!   *are* statements: the top level and a block body. A `# arity-format skip`
//!   between two call arguments marks nothing. (Its lint half, if it has one,
//!   still works — the linter attaches by node, not by line.)
//! - An `# arity-lint on` with no region open. It closes nothing, which usually
//!   means the matching `off` was written with a different prefix: `# arity off`
//!   and `# arity-lint off` open separate regions.
//!
//! Report-only. Moving someone's comment means guessing which statement they
//! meant, and a wrong guess would silently start skipping code they never
//! marked.
//!
//! The placement question is answered by the formatter itself
//! (`arity_formatter::formatter::directive::is_honored_position`) rather than
//! re-derived here, so the report cannot drift from the behavior it describes.

use arity_formatter::formatter::directive::is_honored_position;

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::linter::suppression::{Directive, Verb};
use crate::syntax::SyntaxKind;

pub struct MisplacedSuppression;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "The formatter acts on whole statements, so a directive between \
two arguments marks nothing:",
        source: "f(\n  a = 1,\n  # arity-format skip: hand-aligned\n  b = 2\n)\n",
    },
    Example {
        caption: "An `on` closes only a region opened with the same prefix, so this \
one closes nothing:",
        source: "# arity off\nx <- 1\n# arity-lint on\ny <- 2\n",
    },
];

impl Rule for MisplacedSuppression {
    fn id(&self) -> &'static str {
        "misplaced-suppression"
    }

    fn description(&self) -> &'static str {
        "Flags an `# arity` directive written where it can never take effect. \
A `# arity-format` directive is honored in statement lists — the top level and \
a block body — because that is where the formatter can splice source back \
verbatim; between two call arguments it marks nothing. An `# arity-lint on` \
with no open region closes nothing, which usually means its `off` was written \
with a different prefix (`# arity off` and `# arity-lint off` are separate \
regions). Both fail silently: a directive that does nothing looks exactly like \
one that worked. Report-only — moving the comment would mean guessing which \
statement the author meant."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for directive in ctx.suppressions.directives() {
            if directive.verb == Verb::On {
                if !directive.matched {
                    sink.push(report(
                        directive,
                        "this `on` closes no open region, so it does nothing",
                        "open one first, with the same prefix: `# arity-lint off <rule>: <reason>`",
                    ));
                }
                continue;
            }
            if directive.tool.affects_format()
                && directive.verb != Verb::SkipFile
                && !honored_here(ctx, directive)
            {
                sink.push(report(
                    directive,
                    "the formatter ignores a directive here; it acts on whole statements",
                    "move it above the statement, at the top level or in a block body",
                ));
            }
        }
    }
}

/// Whether the formatter acts on the directive written on this comment.
///
/// Finds the `COMMENT` token by its recorded range and asks the engine. The
/// range came from a `COMMENT` token on this same tree
/// (`SuppressionMap::build`), so descending to the offset finds that token
/// directly — walking the whole tree to look for it would cost this rule
/// O(directives x tree size), which on a directive-dense file is quadratic.
/// The kind and range are still checked, so an offset that somehow lands
/// elsewhere answers exactly as the walk did: not honored.
fn honored_here(ctx: &RuleContext<'_>, directive: &Directive) -> bool {
    // A comment is preceded by a token whenever it is not the file's first
    // byte, so the offset is usually a boundary; the directive's token is the
    // one *starting* there, never the trivia to its left.
    ctx.root
        .token_at_offset(directive.comment.start())
        .right_biased()
        .is_some_and(|token| {
            token.kind() == SyntaxKind::COMMENT
                && token.text_range() == directive.comment
                && is_honored_position(&token)
        })
}

fn report(directive: &Directive, body: &str, suggestion: &str) -> Diagnostic {
    Diagnostic {
        rule: "misplaced-suppression",
        severity: Default::default(),
        path: Default::default(),
        range: directive.comment,
        message: ViolationData::new("misplaced-suppression", body).with_suggestion(suggestion),
        fix: None,
    }
}
