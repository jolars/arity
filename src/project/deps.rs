//! Every way one R file can name a package.
//!
//! The per-file half of the cross-file question `unused-dependency` asks: does
//! *anything* in this package reach the package this `DESCRIPTION` declares?
//! Answering it needs the union over a package's whole R source set, so the
//! per-file part is a range-free `Eq` projection — the backdating-firewall
//! shape the project layer uses everywhere (`.claude/rules/semantic.md`).
//! Editing a function body changes the model but leaves this set equal, so
//! neither the union above it nor the rule re-runs.
//!
//! What counts as reaching a package is deliberately broad, because the
//! consumer reports on *absence*: a missed signal is a maintainer told to
//! delete a dependency their package needs. So the collector over-approximates
//! on purpose and keeps the weakest signal — a bare string that looks like a
//! package name — in [its own field](PackageReferences::string_mentions), where
//! the trade stays visible and revocable.

use std::collections::BTreeSet;

use rowan::ast::AstNode as _;

use crate::ast::{CallExpr, RoxygenBlock};
use crate::semantic::SemanticModel;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// The roxygen tags that name a package, and how many of their words do.
///
/// `@importFrom pkg a b` names one package and then symbols; `@import a b`
/// names only packages. `@rawNamespace` carries arbitrary directive text, so
/// every word in it is treated as a candidate — over-approximating, which for
/// this consumer means staying quiet rather than accusing.
const PACKAGE_TAGS: [(&str, Words); 6] = [
    ("import", Words::All),
    ("importFrom", Words::First),
    ("importClassesFrom", Words::First),
    ("importMethodsFrom", Words::First),
    ("rawNamespace", Words::All),
    ("depends", Words::All),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Words {
    First,
    All,
}

/// The `methods` generics whose presence means a package needs `Imports:
/// methods` with no textual reference to `methods` anywhere.
///
/// R's own check (`tools:::.check_packages_used`) looks for `setClass` and
/// `setMethod`; the other three are the same fact and are included because this
/// list only ever *silences* a finding.
const METHODS_CALLS: [&str; 5] = [
    "setClass",
    "setMethod",
    "setGeneric",
    "setRefClass",
    "setValidity",
];

/// Every package one R file names, as a range-free `Eq` projection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageReferences {
    /// Packages the code reaches directly: the left of every `::` / `:::`, and
    /// the package argument of every `library` / `require` / `requireNamespace`
    /// / `loadNamespace` call — **at any depth**.
    ///
    /// Depth matters: `requireNamespace()` inside a function body is *the*
    /// conditional-dependency idiom, and `SemanticModel::loaded_packages` is
    /// deliberately top-level-only because it models *attachment*, a narrower
    /// fact. Reading this off attachment would turn the most careful way to
    /// depend on a package into a report that it is unused.
    pub direct: BTreeSet<String>,
    /// Packages named by a roxygen `@import`/`@importFrom`/`@importClassesFrom`
    /// /`@importMethodsFrom`/`@rawNamespace` tag.
    ///
    /// Redundant with NAMESPACE *when NAMESPACE is current* — and the point is
    /// that it may not be: mid-`document()`, right after `use_package()`, or in
    /// a package whose NAMESPACE is generated at build time.
    pub roxygen_imports: BTreeSet<String>,
    /// Every string literal shaped like a package name. **Not a reference** —
    /// the over-approximation a consumer consults so a dynamic use
    /// (`do.call("::", …)`, `requireNamespace(pkg_name)`,
    /// `system.file(package = "pkg")`, a "please install pkg" message) can
    /// never produce a false report of disuse.
    pub string_mentions: BTreeSet<String>,
    /// Whether the file defines an S4 or reference class or method — R's own
    /// `uses_methods` flag, which makes `Imports: methods` mandatory with no
    /// textual reference to `methods` anywhere.
    pub uses_methods: bool,
}

/// Derive [`PackageReferences`] for one file.
///
/// Pure, and deliberately in the project layer rather than in a rule: it walks
/// the tree, and no lint rule may do that outside the driver's shared walk.
pub fn file_package_references(root: &SyntaxNode, model: &SemanticModel) -> PackageReferences {
    let mut refs = PackageReferences {
        direct: model
            .referenced_packages()
            .iter()
            .map(|pkg| pkg.to_string())
            .chain(
                model
                    .loaded_packages()
                    .iter()
                    .map(|pkg| pkg.name.to_string()),
            )
            .collect(),
        ..Default::default()
    };

    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::ROXYGEN_BLOCK => {
                let Some(block) = RoxygenBlock::cast(node.clone()) else {
                    continue;
                };
                collect_roxygen_imports(&block, &mut refs.roxygen_imports);
            }
            SyntaxKind::CALL_EXPR => {
                if let Some(call) = CallExpr::cast(node.clone())
                    && call
                        .callee_name()
                        .is_some_and(|name| METHODS_CALLS.contains(&name.as_str()))
                {
                    refs.uses_methods = true;
                }
            }
            _ => {}
        }
    }

    for token in root
        .descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .filter(|token| token.kind() == SyntaxKind::STRING)
    {
        let text = token.text();
        let inner = text
            .strip_prefix(['"', '\''])
            .and_then(|rest| rest.get(..rest.len().saturating_sub(1)))
            .unwrap_or(text);
        if is_package_shaped(inner) {
            refs.string_mentions.insert(inner.to_string());
        }
    }

    refs
}

fn collect_roxygen_imports(block: &RoxygenBlock, out: &mut BTreeSet<String>) {
    for section in block.sections() {
        let Some(tag) = section.tag() else {
            continue;
        };
        let Some(name) = tag.name() else {
            continue;
        };
        let Some((_, words)) = PACKAGE_TAGS
            .iter()
            .find(|(known, _)| *known == name.as_str())
        else {
            continue;
        };
        // The parser splits an arg-bearing tag into `arg` + `text`, and which
        // side a word lands on depends on the tag; joining them back is what
        // makes one word-splitting rule cover every shape here.
        let arg = tag.arg().map(|t| t.text().to_string()).unwrap_or_default();
        let text = tag.text().map(|t| t.text().to_string()).unwrap_or_default();
        let joined = format!("{arg} {text}");
        let mut candidates = joined
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '"' | '\''))
            .filter(|word| is_package_shaped(word));
        match words {
            Words::First => out.extend(candidates.next().map(str::to_string)),
            Words::All => out.extend(candidates.map(str::to_string)),
        }
    }
}

/// Whether `name` could be an R package name: R requires at least two
/// characters, starting with a letter, and only letters, digits, and dots.
fn is_package_shaped(name: &str) -> bool {
    name.len() >= 2
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(source: &str) -> PackageReferences {
        let parsed = crate::parser::parse(source);
        let model = SemanticModel::build(&parsed.cst);
        file_package_references(&parsed.cst, &model)
    }

    #[test]
    fn qualified_access_is_direct() {
        assert!(refs("dplyr::filter(x)\n").direct.contains("dplyr"));
        assert!(refs("dplyr:::internal(x)\n").direct.contains("dplyr"));
    }

    #[test]
    fn attaching_is_direct() {
        assert!(refs("library(dplyr)\n").direct.contains("dplyr"));
        assert!(refs("require(dplyr)\n").direct.contains("dplyr"));
    }

    /// The conditional-dependency idiom. `loaded_packages` is top-level-only,
    /// so reading this off attachment would miss the most careful way to depend
    /// on a package.
    #[test]
    fn a_load_inside_a_function_body_is_direct() {
        let source = "f <- function() {\n  requireNamespace(\"dplyr\")\n}\n";
        assert!(refs(source).direct.contains("dplyr"));
        assert!(
            refs("f <- function() loadNamespace(\"dplyr\")\n")
                .direct
                .contains("dplyr")
        );
    }

    #[test]
    fn roxygen_import_tags_name_packages() {
        let source = "#' @importFrom dplyr filter select\nf <- function() 1\n";
        let found = refs(source);
        assert!(found.roxygen_imports.contains("dplyr"));
        // Only the first word of `@importFrom` is a package.
        assert!(!found.roxygen_imports.contains("filter"));
    }

    #[test]
    fn roxygen_import_names_every_package() {
        let source = "#' @import dplyr rlang\nf <- function() 1\n";
        let found = refs(source);
        assert!(found.roxygen_imports.contains("dplyr"));
        assert!(found.roxygen_imports.contains("rlang"));
    }

    #[test]
    fn s4_definitions_use_methods() {
        assert!(refs("setClass(\"A\", representation(x = \"numeric\"))\n").uses_methods);
        assert!(refs("setMethod(\"show\", \"A\", function(object) NULL)\n").uses_methods);
        assert!(!refs("f <- function() 1\n").uses_methods);
    }

    #[test]
    fn package_shaped_strings_are_mentions_not_references() {
        let found = refs("do.call(\"::\", list(\"dplyr\", \"filter\"))\n");
        assert!(found.string_mentions.contains("dplyr"));
        assert!(!found.direct.contains("dplyr"));
    }

    /// A whole string has to look like a package name, so prose does not
    /// silence every dependency the package declares. That also means a package
    /// named only inside a sentence is not a mention — which is right: the
    /// dynamic idioms this guard exists for all pass the name as its own
    /// string.
    #[test]
    fn prose_is_not_a_package_mention() {
        let found = refs("message(\"could not find the file\")\n");
        assert!(found.string_mentions.is_empty(), "{found:?}");
    }

    #[test]
    fn a_package_argument_is_a_mention() {
        let found = refs("system.file(\"extdata\", package = \"dplyr\")\n");
        assert!(found.string_mentions.contains("dplyr"));
    }
}
