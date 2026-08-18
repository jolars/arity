//! Comment-based suppression: the lint half of the `# arity` directive table.
//!
//! ```text
//! # arity-lint skip <rule>: <reason>        the next non-trivia sibling
//! # arity-lint off <rule>: <reason>         until `# arity-lint on`, or end of file
//! # arity-lint skip-file <rule>: <reason>   anywhere in the file
//! ```
//!
//! A `:` where the rule ID would go widens any of the three to every rule, as
//! does the `# arity` spelling, which addresses the formatter at the same time.
//! `# arity-ignore` and `# arity-ignore-file` are the deprecated spellings of
//! `skip` and `skip-file`; they behave identically here.
//!
//! The grammar itself lives in [`crate::directive`] so the formatter and both
//! of arity's grammars read one table. This module is what that table *means*
//! to the linter: where a directive attaches, and which findings it removes.
//!
//! Implementation note: the comment-to-node attachment for a node-level
//! suppression is "next non-trivia sibling", computed during the walk. This
//! avoids the rowan/biome `place_comment` indirection jarl had to write.
//!
//! Every recognized directive is also recorded in [`SuppressionMap::directives`]
//! — *including* the ones that suppress nothing (an unknown rule ID, a directive
//! with no following sibling, one that names no rule at all), and separately the
//! ones that do not parse at all ([`SuppressionMap::malformed`]). Those are
//! exactly what the `meta/*-suppression` rules exist to report, and they reach a
//! rule through `RuleContext::suppressions`.
//!
//! [`SuppressionMap::filter`] additionally reports which directives actually
//! fired. That is a *driver* fact — it does not exist until the findings have
//! been filtered — which is why `outdated-suppression` is a post-pass
//! (`Rule::check_suppressions`) rather than an ordinary `check_file` rule.

use std::collections::HashMap;

use rowan::{NodeOrToken, TextRange, TextSize};

use crate::dcf;
use crate::directive::{self, Parsed, RuleScope};
use crate::syntax::{SyntaxKind, SyntaxNode};

use super::diagnostic::Diagnostic;

pub use crate::directive::{MalformedKind, RuleRef, Spelling, Tool, Verb};

/// What a directive covers, once its position in the file is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// The whole file.
    File,
    /// Exactly this range: the node a `skip` attached to, or the span from an
    /// `off` to its `on`.
    Range(TextRange),
    /// Nothing at all — a `skip` with nothing after it, an `on` that closes no
    /// open region, or a directive addressed only to the formatter.
    Nothing,
}

/// One parsed `# arity…` comment, placed in the file.
///
/// The ranges inside [`Directive::scope`] are absolute, unlike the relative ones
/// [`crate::directive::parse`] returns.
#[derive(Debug, Clone)]
pub struct Directive {
    pub tool: Tool,
    pub verb: Verb,
    /// The tool prefix as written; see [`crate::directive::Directive::prefix`].
    pub prefix: TextRange,
    pub scope: RuleScope,
    pub spelling: Spelling,
    /// The text after the `:` that follows the rule ID, trimmed. `None` when
    /// there is no `:` at all, or nothing but whitespace follows it.
    pub reason: Option<String>,
    /// The `COMMENT` token's own range — the span a meta rule reports on.
    pub comment: TextRange,
    pub coverage: Coverage,
    /// Whether an `off` found its `on`, or an `on` closed an open region.
    /// Meaningless (and `false`) for the other verbs.
    pub matched: bool,
    /// The comment's text, verbatim.
    pub raw: String,
}

impl Directive {
    /// The rule the author named, if they named one.
    pub fn rule(&self) -> Option<&RuleRef> {
        match &self.scope {
            RuleScope::Rule(rule) => Some(rule),
            _ => None,
        }
    }

    /// Whether the author wrote a reason for the suppression.
    pub fn has_reason(&self) -> bool {
        self.reason.is_some()
    }

    /// Whether this directive can never suppress anything because nothing
    /// follows it — dead regardless of which rules ran.
    pub fn is_dangling(&self) -> bool {
        self.verb == Verb::Skip && self.coverage == Coverage::Nothing
    }

    /// Whether the rule slot means anything here; see
    /// [`crate::directive::Directive::has_rule_slot`].
    pub fn has_rule_slot(&self) -> bool {
        self.tool.affects_lint() && self.verb != Verb::On
    }
}

/// A comment that announced itself as a directive but does not parse as one.
#[derive(Debug, Clone)]
pub struct Malformed {
    pub tool: Tool,
    pub kind: MalformedKind,
    /// The offending word's absolute range.
    pub range: TextRange,
    pub word: String,
    /// The `COMMENT` token's own range.
    pub comment: TextRange,
}

/// Which directives suppressed at least one finding, parallel to
/// [`SuppressionMap::directives`].
#[derive(Debug, Clone, Default)]
pub struct DirectiveUsage(Vec<bool>);

impl DirectiveUsage {
    /// Whether the directive at `index` suppressed at least one diagnostic.
    pub fn is_used(&self, index: usize) -> bool {
        self.0.get(index).copied().unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SuppressionMap {
    /// Every recognized directive, in source order.
    directives: Vec<Directive>,
    /// Every comment that meant to be a directive and is not.
    malformed: Vec<Malformed>,
    /// Indices of the directives covering every rule.
    all_rules: Vec<usize>,
    /// Rule ID -> indices of the directives naming it.
    by_rule: HashMap<String, Vec<usize>>,
}

impl SuppressionMap {
    pub fn build(root: &SyntaxNode) -> Self {
        let mut map = Self::default();
        for element in root.descendants_with_tokens() {
            if let NodeOrToken::Token(token) = element
                && token.kind() == SyntaxKind::COMMENT
            {
                map.classify(token.text(), token.text_range(), || {
                    next_meaningful_sibling(&token)
                });
            }
        }
        map.finish(root.text_range().end());
        map
    }

    /// Build from a `DESCRIPTION` document: every `COMMENT` token, which in DCF
    /// only ever sits under a `COMMENT_LINE`.
    ///
    /// DCF has no trailing comments — a `#` mid-value is value text — so a
    /// directive is always a line of its own at column zero. What a node-scope
    /// directive attaches to follows from where that line sits: see
    /// [`next_meaningful_dcf_sibling`].
    pub fn build_dcf(root: &dcf::SyntaxNode) -> Self {
        let mut map = Self::default();
        for el in root.descendants_with_tokens() {
            if let NodeOrToken::Token(tok) = el
                && tok.kind() == dcf::SyntaxKind::COMMENT
            {
                map.classify(tok.text(), tok.text_range(), || {
                    next_meaningful_dcf_sibling(&tok)
                });
            }
        }
        map.finish(root.text_range().end());
        map
    }

    /// Every recognized directive in the file, in source order.
    pub fn directives(&self) -> &[Directive] {
        &self.directives
    }

    /// Every comment that announced itself as a directive and does not parse.
    pub fn malformed(&self) -> &[Malformed] {
        &self.malformed
    }

    pub fn is_suppressed(&self, rule: &str, range: TextRange) -> bool {
        self.candidates(rule).any(|i| self.covers(i, range))
    }

    /// Drop the suppressed diagnostics, reporting which directives fired.
    ///
    /// *Every* directive covering a finding is marked used, not just the first:
    /// a file-wide and a node directive for the same rule are both doing their
    /// job, and marking only one would leave the other looking outdated.
    ///
    /// Idempotent — a second call over already-filtered diagnostics removes
    /// nothing further, which is what lets the driver re-filter after appending
    /// its post-pass findings.
    pub fn filter(&self, diagnostics: &mut Vec<Diagnostic>) -> DirectiveUsage {
        let mut used = vec![false; self.directives.len()];
        let mut hits = Vec::new();
        diagnostics.retain(|d| {
            hits.clear();
            hits.extend(self.candidates(d.rule).filter(|&i| self.covers(i, d.range)));
            for &i in &hits {
                used[i] = true;
            }
            hits.is_empty()
        });
        DirectiveUsage(used)
    }

    /// Every directive that could suppress `rule`: the blanket ones and the ones
    /// naming it.
    fn candidates<'a>(&'a self, rule: &str) -> impl Iterator<Item = usize> + 'a {
        self.all_rules
            .iter()
            .copied()
            .chain(self.by_rule.get(rule).into_iter().flatten().copied())
    }

    /// Whether directive `index` covers a finding at `range`.
    ///
    /// A *blanket* directive never suppresses a finding spanned on a directive
    /// comment — its own or any other's. Without that, `# arity-lint skip-file:`
    /// would suppress the `blanket-suppression` finding about itself, and an
    /// unclosed `# arity off` would suppress the `misplaced-suppression` finding
    /// about the `on` that failed to close it: the two rules would be
    /// structurally unreportable in the cases they exist for. A directive that
    /// *names* a meta rule still silences it, which is how an author says "I
    /// know". The exemption is inert for every non-`meta` rule: no other rule's
    /// finding is ever spanned on a comment.
    fn covers(&self, index: usize, range: TextRange) -> bool {
        let directive = &self.directives[index];
        if directive.scope == RuleScope::All && self.spans_a_directive(range) {
            return false;
        }
        match directive.coverage {
            Coverage::File => true,
            Coverage::Range(covered) => covered.contains_range(range),
            Coverage::Nothing => false,
        }
    }

    /// Whether `range` lies inside some directive's own comment — recognized or
    /// malformed, since both are reported by the `meta` rules.
    fn spans_a_directive(&self, range: TextRange) -> bool {
        self.directives
            .iter()
            .map(|d| d.comment)
            .chain(self.malformed.iter().map(|m| m.comment))
            .any(|comment| comment.contains_range(range))
    }

    /// Parse one comment and record what it is, grammar-free.
    ///
    /// `target` is invoked only for the node-scope form, so the common case (a
    /// file-scope directive, and every comment that is not a directive at all)
    /// never pays for the sibling walk. That laziness is why both grammars can
    /// share this without sharing a tree type.
    fn classify(
        &mut self,
        text: &str,
        comment: TextRange,
        target: impl FnOnce() -> Option<TextRange>,
    ) {
        let base = comment.start();
        match directive::parse(text) {
            Some(Parsed::Directive(parsed)) => {
                let coverage = match parsed.verb {
                    Verb::SkipFile => Coverage::File,
                    Verb::Skip => target().map_or(Coverage::Nothing, Coverage::Range),
                    // Resolved against the matching `on` once the walk is done.
                    Verb::Off | Verb::On => Coverage::Nothing,
                };
                self.directives.push(Directive {
                    tool: parsed.tool,
                    verb: parsed.verb,
                    prefix: parsed.prefix + base,
                    scope: absolute_scope(parsed.scope, base),
                    spelling: parsed.spelling,
                    reason: parsed.reason.map(str::to_string),
                    comment,
                    coverage,
                    matched: false,
                    raw: text.to_string(),
                });
            }
            Some(Parsed::Malformed(bad)) => self.malformed.push(Malformed {
                tool: bad.tool,
                kind: bad.kind,
                range: bad.range + base,
                word: bad.word,
                comment,
            }),
            None => {}
        }
    }

    /// Close the regions and index everything. Runs once, after the walk, since
    /// an `off` cannot know where it ends until its `on` has been seen.
    fn finish(&mut self, end_of_file: TextSize) {
        self.close_regions(end_of_file);
        for index in 0..self.directives.len() {
            let directive = &self.directives[index];
            if !directive.tool.affects_lint() || directive.verb == Verb::On {
                continue;
            }
            match &directive.scope {
                RuleScope::All => self.all_rules.push(index),
                RuleScope::Rule(rule) => {
                    self.by_rule.entry(rule.id.clone()).or_default().push(index);
                }
                // Names nothing, so it can match nothing. Recorded all the same:
                // that is what `blanket-suppression` reports.
                RuleScope::Unnamed => {}
            }
        }
    }

    /// Give every `off` the span it covers: from its own comment to the first
    /// `on` written with the same prefix, or to end of file.
    ///
    /// One `on` closes *every* region its prefix has open — there is no nesting
    /// to unwind, and "close the innermost" would silently leave the outer one
    /// running.
    fn close_regions(&mut self, end_of_file: TextSize) {
        let ends: Vec<(usize, Option<usize>)> = self
            .directives
            .iter()
            .enumerate()
            .filter(|(_, d)| d.verb == Verb::Off)
            .map(|(i, off)| {
                let closer = self.directives[i + 1..]
                    .iter()
                    .position(|d| d.verb == Verb::On && d.tool == off.tool)
                    .map(|offset| i + 1 + offset);
                (i, closer)
            })
            .collect();

        for (index, closer) in ends {
            let end = match closer {
                Some(j) => {
                    self.directives[j].matched = true;
                    self.directives[j].comment.start()
                }
                None => end_of_file,
            };
            let start = self.directives[index].comment.end();
            self.directives[index].matched = closer.is_some();
            if start <= end {
                self.directives[index].coverage = Coverage::Range(TextRange::new(start, end));
            }
        }
    }
}

/// Re-anchor a scope's ranges from comment-relative to absolute.
fn absolute_scope(scope: RuleScope, base: TextSize) -> RuleScope {
    match scope {
        RuleScope::Rule(rule) => RuleScope::Rule(RuleRef {
            id: rule.id,
            range: rule.range + base,
        }),
        other => other,
    }
}

/// What a node-scope directive in a `DESCRIPTION` attaches to: the next thing,
/// same as in R, but "next" has to be read off a tree where a comment line is a
/// child of the field it follows.
///
/// A `COMMENT_LINE` lands under whatever node is open, and a `FIELD` stays open
/// across its continuation lines. So a comment between two fields is a child of
/// the *earlier* one — which is not what its author meant. The distinction that
/// matters is therefore whether a value line still follows inside that field:
///
/// - **A value line follows** — the comment interrupts a multi-line value, the
///   case `read.dcf` skips before *resuming* the field. It attaches to the
///   whole enclosing field, which is the only useful answer: a finding on one
///   dependency entry lies inside that field's range.
/// - **Otherwise** — the comment is trailing, and points past its field at
///   whatever line comes next.
///
/// `None` when nothing follows, which leaves the directive dangling exactly as
/// in R. A comment never opens a record, so it can never bridge two.
fn next_meaningful_dcf_sibling(tok: &dcf::SyntaxToken) -> Option<TextRange> {
    let line = tok.parent()?;
    let field = line.parent().filter(|p| p.kind() == dcf::SyntaxKind::FIELD);
    if let Some(field) = field {
        let interrupts_value = line
            .siblings(rowan::Direction::Next)
            .skip(1)
            .any(|sibling| sibling.kind() == dcf::SyntaxKind::VALUE_LINE);
        if interrupts_value {
            return Some(field.text_range());
        }
        return next_dcf_sibling_of(&field);
    }
    next_dcf_sibling_of(&line)
}

/// The range a directive sitting immediately before `node`'s successor covers.
fn next_dcf_sibling_of(node: &dcf::SyntaxNode) -> Option<TextRange> {
    let next = next_dcf_line(node.siblings(rowan::Direction::Next).skip(1))?;
    if next.kind() == dcf::SyntaxKind::RECORD {
        return Some(next_dcf_line(next.children())?.text_range());
    }
    Some(next.text_range())
}

/// The first line node in `lines` that is neither a comment nor blank.
fn next_dcf_line(mut lines: impl Iterator<Item = dcf::SyntaxNode>) -> Option<dcf::SyntaxNode> {
    lines.find(|node| {
        !matches!(
            node.kind(),
            dcf::SyntaxKind::COMMENT_LINE | dcf::SyntaxKind::BLANK_LINE
        )
    })
}

fn next_meaningful_sibling(
    tok: &rowan::SyntaxToken<crate::syntax::RLanguage>,
) -> Option<TextRange> {
    // The "next sibling" is the next non-trivia, non-comment element after
    // this token within its parent node. We expand outward if the parent is
    // itself trivia-only — e.g. a comment between top-level statements lives
    // under ROOT, and the next sibling is the next top-level expression.
    let mut current_token = tok.clone();
    loop {
        let parent = current_token.parent()?;
        let mut found = None;
        let mut past_self = false;
        for el in parent.children_with_tokens() {
            match &el {
                NodeOrToken::Token(t) if *t == current_token => {
                    past_self = true;
                    continue;
                }
                _ => {}
            }
            if !past_self {
                continue;
            }
            match &el {
                NodeOrToken::Token(t)
                    if matches!(
                        t.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    ) =>
                {
                    continue;
                }
                NodeOrToken::Node(child) => {
                    found = Some(child.text_range());
                    break;
                }
                NodeOrToken::Token(t) => {
                    found = Some(t.text_range());
                    break;
                }
            }
        }
        if let Some(range) = found {
            return Some(range);
        }
        // No sibling after this token in `parent`. Bubble up: look for the
        // next non-trivia sibling of `parent` itself.
        let parent_node = parent.clone();
        let grand = parent_node.parent()?;
        let mut past_parent = false;
        for el in grand.children_with_tokens() {
            match &el {
                NodeOrToken::Node(n) if *n == parent_node => {
                    past_parent = true;
                    continue;
                }
                _ => {}
            }
            if !past_parent {
                continue;
            }
            match &el {
                NodeOrToken::Token(t)
                    if matches!(
                        t.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    ) =>
                {
                    continue;
                }
                NodeOrToken::Node(child) => return Some(child.text_range()),
                NodeOrToken::Token(t) => return Some(t.text_range()),
            }
        }
        // Try one level higher.
        current_token = grand.first_token()?;
        // Prevent infinite loops.
        if grand == parent {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn map_of(src: &str) -> SuppressionMap {
        let parsed = parse(src);
        SuppressionMap::build(&parsed.cst)
    }

    fn only(map: &SuppressionMap) -> &Directive {
        match map.directives() {
            [d] => d,
            other => panic!("expected exactly one directive, got {other:?}"),
        }
    }

    #[test]
    fn file_all_suppresses_everything() {
        let m = map_of("# arity-ignore-file: noisy\nx <- 1\n");
        assert!(m.is_suppressed("anything", TextRange::new(26.into(), 32.into())));
    }

    #[test]
    fn file_rule_suppresses_only_that_rule() {
        let m = map_of("# arity-ignore-file unused-binding: temp\nx <- 1\n");
        assert!(m.is_suppressed("unused-binding", TextRange::new(41.into(), 47.into())));
        assert!(!m.is_suppressed("undefined-symbol", TextRange::new(41.into(), 47.into())));
    }

    #[test]
    fn node_suppression_attaches_to_next_sibling() {
        let src = "# arity-ignore unused-binding: temp\nx <- 1\n";
        let m = map_of(src);
        // The `x <- 1` ASSIGNMENT_EXPR spans 36..42 in the file.
        assert!(m.is_suppressed("unused-binding", TextRange::new(36.into(), 42.into())));
    }

    #[test]
    fn node_suppression_does_not_leak_to_following_statements() {
        let src = "# arity-ignore unused-binding: only first\nx <- 1\ny <- 2\n";
        let m = map_of(src);
        // x assignment is at 42..48, y at 49..55.
        assert!(m.is_suppressed("unused-binding", TextRange::new(42.into(), 48.into())));
        assert!(!m.is_suppressed("unused-binding", TextRange::new(49.into(), 55.into())));
    }

    #[test]
    fn directive_records_rule_reason_and_comment_range() {
        let src = "# arity-ignore unused-binding: still needed\nx <- 1\n";
        let m = map_of(src);
        let d = only(&m);
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::Skip));
        assert_eq!(d.rule().map(|r| r.id.as_str()), Some("unused-binding"));
        assert_eq!(d.reason.as_deref(), Some("still needed"));
        assert_eq!(d.comment, TextRange::new(0.into(), 43.into()));
        assert_eq!(d.raw, "# arity-ignore unused-binding: still needed");
        assert_eq!(d.spelling, Spelling::Deprecated);
        assert!(matches!(d.coverage, Coverage::Range(_)));
        assert!(!d.is_dangling());
    }

    #[test]
    fn rule_ref_range_spans_exactly_the_written_id() {
        let src = "# arity-ignore unused-binding: r\nx <- 1\n";
        let m = map_of(src);
        let rule = only(&m).rule().expect("a rule ref").clone();
        assert_eq!(&src[rule.range], "unused-binding");
    }

    #[test]
    fn rule_ref_range_is_absolute_for_an_indented_file_directive() {
        let src = "f <- function() {\n  # arity-ignore-file browser: r\n  1\n}\n";
        let m = map_of(src);
        let rule = only(&m).rule().expect("a rule ref").clone();
        assert_eq!(&src[rule.range], "browser");
    }

    #[test]
    fn directive_without_colon_has_no_reason() {
        let m = map_of("# arity-ignore unused-binding\nx <- 1\n");
        assert!(!only(&m).has_reason());
    }

    #[test]
    fn directive_with_empty_reason_has_no_reason() {
        let m = map_of("# arity-ignore unused-binding:   \nx <- 1\n");
        assert!(!only(&m).has_reason());
    }

    #[test]
    fn blanket_file_directive_has_no_rule_but_keeps_its_reason() {
        let m = map_of("# arity-ignore-file: generated code\nx <- 1\n");
        let d = only(&m);
        assert_eq!(d.verb, Verb::SkipFile);
        assert_eq!(d.scope, RuleScope::All);
        assert_eq!(d.reason.as_deref(), Some("generated code"));
        assert_eq!(d.coverage, Coverage::File);
    }

    #[test]
    fn scoped_file_directive_keeps_rule_and_reason() {
        let m = map_of("# arity-ignore-file unused-binding: temp\nx <- 1\n");
        let d = only(&m);
        assert_eq!(d.verb, Verb::SkipFile);
        assert_eq!(d.rule().map(|r| r.id.as_str()), Some("unused-binding"));
        assert_eq!(d.reason.as_deref(), Some("temp"));
    }

    #[test]
    fn bare_directive_naming_no_rule_is_recorded() {
        // Suppresses nothing today, and did so silently before it was recorded.
        let m = map_of("# arity-ignore\nx <- 1\n");
        let d = only(&m);
        assert_eq!(d.verb, Verb::Skip);
        assert_eq!(d.scope, RuleScope::Unnamed);
    }

    #[test]
    fn node_directive_with_nothing_after_it_is_still_recorded() {
        let m = map_of("x <- 1\n# arity-ignore unused-binding: dangling\n");
        let d = only(&m);
        assert_eq!(d.verb, Verb::Skip);
        assert_eq!(d.coverage, Coverage::Nothing);
        assert!(d.is_dangling());
    }

    #[test]
    fn unknown_rule_id_is_recorded() {
        let m = map_of("# arity-ignore not-a-rule: r\nx <- 1\n");
        assert_eq!(only(&m).rule().map(|r| r.id.as_str()), Some("not-a-rule"));
    }

    #[test]
    fn comma_list_yields_a_single_bogus_rule() {
        // Pins the existing parse: `parse_rule` stops at whitespace, so the
        // comma rides along and the directive silently suppresses nothing.
        // `misnamed-suppression` is what makes this audible.
        let m = map_of("# arity-ignore browser, repeat: r\nx <- 1\n");
        assert_eq!(only(&m).rule().map(|r| r.id.as_str()), Some("browser,"));
    }

    #[test]
    fn non_directive_comments_are_not_recorded() {
        let m = map_of("# just a comment\n#' @param x roxygen\nx <- 1\n");
        assert!(m.directives().is_empty());
    }

    #[test]
    fn filter_reports_which_directives_fired() {
        let src = "# arity-ignore unused-binding: used\n# arity-ignore browser: unused\nx <- 1\n";
        let m = map_of(src);
        let target = range_of(src, "x <- 1");
        let mut diagnostics = vec![diag("unused-binding", target), diag("equals-na", target)];
        let usage = m.filter(&mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, "equals-na");
        assert!(usage.is_used(0));
        assert!(!usage.is_used(1));
    }

    #[test]
    fn filter_marks_every_directive_that_covers_a_finding() {
        let src = "# arity-ignore-file unused-binding: broad\n# arity-ignore unused-binding: narrow\nx <- 1\n";
        let m = map_of(src);
        let target = range_of(src, "x <- 1");
        let mut diagnostics = vec![diag("unused-binding", target)];
        let usage = m.filter(&mut diagnostics);
        assert!(diagnostics.is_empty());
        assert!(usage.is_used(0), "the file-wide directive fired");
        assert!(usage.is_used(1), "the node directive covers it too");
    }

    #[test]
    fn directive_never_suppresses_a_finding_inside_itself() {
        // Otherwise `blanket-suppression` could never report the very shape it
        // exists for.
        let src = "# arity-ignore-file: shush\nx <- 1\n";
        let m = map_of(src);
        let own = only(&m).comment;
        assert!(!m.is_suppressed("blanket-suppression", own));
        // …but it still suppresses everything else in the file.
        assert!(m.is_suppressed("unused-binding", TextRange::new(27.into(), 33.into())));
    }

    #[test]
    fn a_region_covers_from_off_to_on() {
        let src = "x <- 1\n# arity-lint off browser: r\ny <- 2\n# arity-lint on\nz <- 3\n";
        let m = map_of(src);
        assert!(m.is_suppressed("browser", range_of(src, "y <- 2")));
        assert!(!m.is_suppressed("browser", range_of(src, "x <- 1")));
        assert!(!m.is_suppressed("browser", range_of(src, "z <- 3")));
        assert!(m.directives()[0].matched, "the `off` found its `on`");
        assert!(m.directives()[1].matched, "the `on` closed a region");
    }

    #[test]
    fn an_unclosed_region_runs_to_end_of_file() {
        let src = "x <- 1\n# arity-lint off browser: r\ny <- 2\n";
        let m = map_of(src);
        assert!(m.is_suppressed("browser", range_of(src, "y <- 2")));
        assert!(!m.directives()[0].matched);
    }

    #[test]
    fn an_on_with_nothing_open_covers_nothing() {
        let m = map_of("# arity-lint on\nx <- 1\n");
        let d = only(&m);
        assert_eq!(d.verb, Verb::On);
        assert_eq!(d.coverage, Coverage::Nothing);
        assert!(!d.matched);
    }

    #[test]
    fn a_region_closes_only_on_its_own_prefix() {
        // `# arity off` and `# arity-lint off` are different regions.
        let src = "# arity off\nx <- 1\n# arity-lint on\ny <- 2\n";
        let m = map_of(src);
        assert!(!m.directives()[0].matched);
        assert!(
            m.is_suppressed("browser", range_of(src, "y <- 2")),
            "runs on to EOF"
        );
    }

    #[test]
    fn a_format_only_directive_suppresses_no_lint_finding() {
        let src = "# arity-format skip: layout only\nx <- 1\n";
        let m = map_of(src);
        assert_eq!(only(&m).tool, Tool::Format);
        assert!(!m.is_suppressed("unused-binding", range_of(src, "x <- 1")));
    }

    #[test]
    fn a_malformed_directive_is_recorded_separately() {
        let src = "# arity-format skipp: typo\nx <- 1\n";
        let m = map_of(src);
        assert!(m.directives().is_empty());
        let [bad] = m.malformed() else {
            panic!("expected one malformed directive, got {:?}", m.malformed())
        };
        assert_eq!(bad.kind, MalformedKind::UnknownVerb);
        assert_eq!(&src[bad.range], "skipp");
    }

    /// The range of `needle`'s first occurrence in `src`.
    fn range_of(src: &str, needle: &str) -> TextRange {
        let start = src.find(needle).expect("needle in src");
        TextRange::at((start as u32).into(), (needle.len() as u32).into())
    }

    fn diag(rule: &'static str, range: TextRange) -> Diagnostic {
        Diagnostic {
            rule,
            severity: Default::default(),
            path: Default::default(),
            range,
            message: crate::linter::diagnostic::ViolationData::new(rule, "test"),
            fix: None,
        }
    }
}
