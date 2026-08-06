//! `download-file`: `download.file()` used with a non-portable `mode`.
//!
//! `download.file()` defaults to `mode = "w"` — *text* mode. On Windows that
//! translates line endings, so any binary payload (a `.zip`, a `.rds`, a package
//! tarball) arrives corrupted; the same call is silently fine on Unix, which is
//! what makes it a portability bug rather than an obvious one. R's own
//! documentation recommends `mode = "wb"` (or `"ab"` to append).
//!
//! The rule reports three mutually exclusive shapes, mirroring lintr's
//! `download_file_linter`:
//!
//! 1. **Implicit mode** — no `mode` argument, so the text-mode default applies.
//! 2. **Explicit text mode** — `mode = "w"` / `mode = "a"` written out.
//! 3. **Ignored mode** — a `mode` supplied alongside `method = "curl"` /
//!    `"wget"`, which shell out to an external downloader and ignore it. The
//!    argument reads as a portability guarantee it does not provide.
//!
//! Arguments are resolved through R's real matching rules
//! ([`match_args_to_formals`] over `download.file`'s formals), so a positional
//! `method` (`download.file(u, d, "curl")`) and a unique-prefix partial match
//! (`mod = "wb"`) both land on the right formal.
//!
//! **Namespace-confirmed** (`ns`): the callee must resolve to base R
//! ([`RuleContext::resolves_to_base`]), so a local redefinition or a qualified
//! `utils::download.file(...)` is left alone.
//!
//! **Conservative on anything unknowable.** A `mode` or `method` that is not a
//! plain string literal (`mode = m`, `method = getOption("dl.method")`) could be
//! any value, so the call is skipped rather than guessed at — this rule is
//! report-only, so over-suppressing is the safe direction. A value-less `ARG` (a
//! stray comment between commas, an empty slot) would shift positional matching,
//! so it likewise skips the call.
//!
//! **No autofix.** The implicit-mode shape needs an argument *inserted*, and the
//! ignored-mode shape needs one deleted along with its separator — neither is a
//! tight, trivia-safe textual edit. Rewriting only the `mode = "w"` -> `"wb"`
//! shape would fix one case in three, so the rule stays uniformly report-only.
//!
//! [`match_args_to_formals`]: crate::semantic::match_args_to_formals
//! [`RuleContext::resolves_to_base`]: crate::linter::rules::RuleContext::resolves_to_base

use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, CallExpr, HasArgList as _};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::semantic::match_args_to_formals;
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct DownloadFile;

/// `download.file`'s formals, in declaration order, for [`match_args_to_formals`].
const FORMALS: &[&str] = &[
    "url", "destfile", "method", "quiet", "mode", "cacheOK", "extra", "headers", "...",
];

/// The `method` values that shell out to an external downloader, which ignores
/// whatever `mode` was asked for.
const METHODS_IGNORING_MODE: &[&str] = &["curl", "wget"];

const EXAMPLES: &[Example] = &[
    Example {
        caption: "Relying on the default `mode = \"w\"`, which corrupts a binary \
                  download on Windows:",
        source: "download.file(url, destfile)\n",
    },
    Example {
        caption: "`mode` is ignored by `method = \"curl\"` and `method = \"wget\"`:",
        source: "download.file(url, destfile, method = \"curl\", mode = \"wb\")\n",
    },
];

impl Rule for DownloadFile {
    fn id(&self) -> &'static str {
        "download-file"
    }

    fn description(&self) -> &'static str {
        "Flag a `download.file()` call whose `mode` is not portable.\n\nThe \
         default `mode = \"w\"` is text mode: on Windows it translates line \
         endings, corrupting any binary payload, while the same call works on \
         Unix. R recommends `mode = \"wb\"` (or `\"ab\"` to append), so the rule \
         reports an omitted `mode`, an explicit `mode = \"w\"` / `\"a\"`, and a \
         `mode` supplied next to `method = \"curl\"` / `\"wget\"` (which shell \
         out and ignore it).\n\nArguments are matched the way R matches them, so \
         a positional or partially-named `method`/`mode` is understood. The \
         callee must resolve to base R, and a `mode`/`method` that is not a \
         string literal is skipped rather than guessed at. There is no autofix: \
         the shapes need an argument inserted or deleted, not rewritten."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        if matchers::callee_name(&call).as_deref() != Some("download.file") {
            return;
        }
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let args: Vec<Arg> = call.args().collect();
        // A value-less `ARG` (a stray comment, an empty slot) would shift the
        // positional fill, so no verdict is possible.
        if args.iter().any(|a| a.value().is_none()) {
            return;
        }
        let names: Vec<Option<SmolStr>> = args.iter().map(Arg::name).collect();
        let matched = match_args_to_formals(&names, FORMALS);
        let arg_for = |formal: &str| {
            args.iter()
                .zip(&matched)
                .find(|(_, m)| **m == Some(formal))
                .map(|(a, _)| a)
        };

        // A supplied-but-unreadable `mode`/`method` could be anything; skip.
        let (mode_arg, mode) = match arg_for("mode") {
            Some(arg) => match string_value(arg) {
                Some(value) => (Some(arg), Some(value)),
                None => return,
            },
            None => (None, None),
        };
        let method = match arg_for("method") {
            Some(arg) => match string_value(arg) {
                Some(value) => Some(value),
                None => return,
            },
            None => None,
        };
        let ignored = method
            .as_deref()
            .is_some_and(|m| METHODS_IGNORING_MODE.contains(&m));

        let (range, body, suggestion) = match (mode.as_deref(), ignored) {
            // No `mode`, and the method honors it: the text-mode default applies.
            // Span the callee — there is no argument to point at.
            (None, false) => (
                call.callee_token()
                    .map_or_else(|| call.syntax().text_range(), |t| t.text_range()),
                "`download.file()` relies on the default `mode = \"w\"`, which corrupts \
                 binary downloads on Windows"
                    .to_string(),
                "Pass `mode = \"wb\"` (or `mode = \"ab\"` to append).".to_string(),
            ),
            // No `mode` and a method that would ignore one: exactly right.
            (None, true) => return,
            (Some(_), true) => {
                let arg = mode_arg.expect("an explicit mode has an argument");
                let method = method.as_deref().unwrap_or_default();
                (
                    arg.syntax().text_range(),
                    format!("`mode` is ignored by `download.file(method = \"{method}\")`"),
                    format!(
                        "Drop the `mode` argument, or use a `method` that honors it \
                         (`\"{method}\"` shells out to an external downloader)."
                    ),
                )
            }
            (Some(mode @ ("w" | "a")), false) => {
                let arg = mode_arg.expect("an explicit mode has an argument");
                (
                    arg.syntax().text_range(),
                    format!(
                        "`mode = \"{mode}\"` is text mode, which corrupts binary downloads \
                         on Windows"
                    ),
                    format!("Use `mode = \"{mode}b\"`."),
                )
            }
            // An explicit binary (or otherwise deliberate) mode.
            (Some(_), false) => return,
        };

        sink.push(Diagnostic {
            rule: "download-file",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new("download-file", body).with_suggestion(suggestion),
            fix: None,
        });
    }
}

/// The contents of `arg`'s value when it is a plain single-token string literal
/// (`"wb"`, `'curl'`). `None` for a computed value, a name, or a raw string —
/// all of which the rule treats as unknowable.
fn string_value(arg: &Arg) -> Option<String> {
    let token = arg.value()?.into_token()?;
    let (_, inner) = matchers::string_literal(&token)?;
    Some(inner.to_string())
}
