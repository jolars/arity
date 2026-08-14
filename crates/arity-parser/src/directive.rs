//! The `# arity…` directive grammar, shared by the formatter and the linter.
//!
//! A directive is an ordinary `#` comment that tells one or both tools to stand
//! down. One grammar spans them:
//!
//! ```text
//! # arity[-format|-lint] <verb> [<rule>][: <reason>]      verb: skip | off | on | skip-file
//! ```
//!
//! which fills a three-by-three table of scope against tool:
//!
//! ```text
//! # arity-format skip: why        # arity-lint skip <rule>: why       # arity skip: why
//! # arity-format off … on         # arity-lint off <rule> … on        # arity off … on
//! # arity-format skip-file: why   # arity-lint skip-file <rule>: why  # arity skip-file: why
//! ```
//!
//! Only a lint directive names a rule: `# arity-format` has no lint half, and
//! `# arity` covers every rule by construction.
//!
//! # Deprecated spellings
//!
//! `# arity-ignore <rule>: why` and `# arity-ignore-file <rule>: why` are what
//! the linter shipped with. They still parse, as [`Verb::Skip`] and
//! [`Verb::SkipFile`] tagged [`Spelling::Deprecated`], so a rule can find and
//! rewrite them; nothing else should treat them differently.
//!
//! This module is the single source of truth for that table. It parses comment
//! *text* — no tree, no grammar — which is what lets the R walk, the `DESCRIPTION`
//! walk, and the formatter share it without sharing a node type.
//!
//! # Offsets
//!
//! Every range is **relative to the start of the comment text** passed in. A
//! caller holding a token adds the token's own start to make it absolute.

use rowan::{TextRange, TextSize};

/// Which tool a directive addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// `# arity …` — both the formatter and the linter.
    Both,
    /// `# arity-format …` — the formatter alone.
    Format,
    /// `# arity-lint …` — the linter alone.
    Lint,
}

impl Tool {
    /// Whether a directive with this tool suppresses lint findings.
    pub fn affects_lint(self) -> bool {
        matches!(self, Tool::Both | Tool::Lint)
    }

    /// Whether a directive with this tool suppresses formatting.
    pub fn affects_format(self) -> bool {
        matches!(self, Tool::Both | Tool::Format)
    }

    /// The canonical prefix, for a diagnostic that suggests a correction.
    pub fn prefix(self) -> &'static str {
        match self {
            Tool::Both => "arity",
            Tool::Format => "arity-format",
            Tool::Lint => "arity-lint",
        }
    }
}

/// Whether a directive is written the canonical way or in one of the spellings
/// the linter shipped with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spelling {
    /// `# arity-lint skip <rule>: why` and friends.
    Canonical,
    /// `# arity-ignore <rule>: why` / `# arity-ignore-file <rule>: why`.
    /// Honored exactly like the canonical form; only a rule that rewrites them
    /// should care about the difference.
    Deprecated,
}

/// What a directive does, and over what span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// The next non-trivia sibling only.
    Skip,
    /// Everything until the matching `on`, or end of file.
    Off,
    /// Closes the open regions opened with the same prefix.
    On,
    /// The whole file.
    SkipFile,
}

impl Verb {
    fn from_word(word: &str) -> Option<Self> {
        match word {
            "skip" => Some(Verb::Skip),
            "off" => Some(Verb::Off),
            "on" => Some(Verb::On),
            "skip-file" => Some(Verb::SkipFile),
            _ => None,
        }
    }

    /// Whether the author owes a reason. `on` closes a region someone else
    /// already justified, so it owes nothing.
    pub fn wants_reason(self) -> bool {
        !matches!(self, Verb::On)
    }
}

/// A rule ID as written, with the range it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRef {
    pub id: String,
    pub range: TextRange,
}

/// A comment that announced itself as a directive but does not parse as one.
///
/// Recorded rather than dropped: a directive's failure mode is that nothing
/// happens, which is also what success looks like, so a malformed one has to be
/// reported or it fails silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    pub tool: Tool,
    pub kind: MalformedKind,
    /// The offending word, or the whole prefix when nothing followed it.
    pub range: TextRange,
    /// The offending word as written.
    pub word: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedKind {
    /// A word in verb position that is not one of the four verbs.
    UnknownVerb,
    /// A tool prefix with no verb after it at all.
    MissingVerb,
    /// Text after the verb that is not a `: reason`. A format directive names
    /// no rule, and the `# arity` form covers every rule by construction, so a
    /// word there was meant to do something it cannot do.
    UnexpectedWord,
    /// A verb after the deprecated `arity-ignore`, which spells its own verb —
    /// a mix of the two spellings.
    UnexpectedVerb,
}

/// Which lint rules a directive covers.
///
/// The three cases are deliberately distinct because two of them look identical
/// in the source and behave in opposite directions: `# arity-lint skip-file:`
/// turns the whole linter off, while a bare `# arity-lint skip-file` turns
/// nothing off at all. Collapsing them would silently promote the inert one
/// into a blanket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleScope {
    /// Exactly the rule named.
    Rule(RuleRef),
    /// Every lint rule. Written as a `:` where the rule ID would go, or implied
    /// by the `# arity` form, whose lint half is all-rules by construction.
    All,
    /// No rule named at all — inert, and reported for exactly that reason.
    Unnamed,
}

/// A recognized directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive<'a> {
    pub tool: Tool,
    pub verb: Verb,
    pub scope: RuleScope,
    /// The text after the `:`, trimmed. `None` when there is no `:`, or nothing
    /// but whitespace follows it.
    pub reason: Option<&'a str>,
    pub spelling: Spelling,
}

impl Directive<'_> {
    /// The rule the author named, if they named one.
    pub fn rule(&self) -> Option<&RuleRef> {
        match &self.scope {
            RuleScope::Rule(rule) => Some(rule),
            _ => None,
        }
    }

    /// Whether the rule slot means anything here. A format-only directive names
    /// no rule because it cannot, and an `on` closes whatever is open rather
    /// than naming anything — neither is an author who forgot.
    pub fn has_rule_slot(&self) -> bool {
        self.tool.affects_lint() && self.verb != Verb::On
    }
}

/// The result of reading one comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed<'a> {
    Directive(Directive<'a>),
    Malformed(Malformed),
}

/// Read one `#` comment as a directive.
///
/// `None` for every comment that is not one — including a comment that merely
/// starts with the word `arity`. A tool prefix (`arity-lint`, `arity-format`,
/// and the deprecated `arity-ignore`/`arity-ignore-file`) is unambiguous intent,
/// so a bad tail there yields [`Parsed::Malformed`]; a bare `arity` is only a
/// directive when a verb follows, because prose about arity is otherwise
/// indistinguishable from a typo'd one.
pub fn parse(text: &str) -> Option<Parsed<'_>> {
    let hash = text.find('#')?;
    let (body, body_at) = trim_start_at(&text[hash + 1..], hash + 1);

    // The deprecated spellings first, longest prefix first: `arity-ignore-file`
    // before `arity-ignore`, and both before the canonical prefixes.
    if let Some((rest, rest_at)) = strip_word(body, body_at, "arity-ignore-file") {
        return Some(Parsed::Directive(lint_tail(
            Verb::SkipFile,
            rest,
            rest_at,
            Spelling::Deprecated,
        )));
    }
    if let Some((rest, rest_at)) = strip_word(body, body_at, "arity-ignore") {
        return Some(arity_ignore(rest, rest_at));
    }
    for (prefix, tool) in [("arity-lint", Tool::Lint), ("arity-format", Tool::Format)] {
        if let Some((rest, rest_at)) = strip_word(body, body_at, prefix) {
            return Some(tool_prefixed(
                tool,
                rest,
                rest_at,
                TextRange::at(size(body_at), size(prefix.len())),
            ));
        }
    }
    if let Some((rest, rest_at)) = strip_word(body, body_at, "arity") {
        // A bare `arity` is directive intent only when a verb follows: prose
        // about arity is otherwise indistinguishable from a typo'd directive.
        Verb::from_word(next_word(rest)?)?;
        return Some(tool_prefixed(
            Tool::Both,
            rest,
            rest_at,
            TextRange::at(size(body_at), size("arity".len())),
        ));
    }
    None
}

/// The deprecated `# arity-ignore …`, which spells its own verb: it *is*
/// `arity-lint skip`, so a verb after it is a mix of the two spellings.
fn arity_ignore(rest: &str, rest_at: usize) -> Parsed<'_> {
    match next_word(rest) {
        Some(word) if Verb::from_word(word).is_some() => Parsed::Malformed(Malformed {
            tool: Tool::Lint,
            kind: MalformedKind::UnexpectedVerb,
            range: TextRange::at(size(rest_at), size(word.len())),
            word: word.to_string(),
        }),
        _ => Parsed::Directive(lint_tail(Verb::Skip, rest, rest_at, Spelling::Deprecated)),
    }
}

/// The tail of a lint directive: a rule slot, then an optional reason.
///
/// A `:` where the rule ID would go is the blanket form; nothing at all is the
/// inert one.
fn lint_tail(verb: Verb, rest: &str, rest_at: usize, spelling: Spelling) -> Directive<'_> {
    let scope = match parse_rule(rest, rest_at) {
        Some(rule) => RuleScope::Rule(rule),
        None if rest.starts_with(':') => RuleScope::All,
        None => RuleScope::Unnamed,
    };
    Directive {
        tool: Tool::Lint,
        verb,
        scope,
        reason: parse_reason(rest),
        spelling,
    }
}

/// A canonical `# arity-lint …`, `# arity-format …`, or `# arity …`: the verb is
/// required, and only a lint directive may name a rule after it.
fn tool_prefixed(tool: Tool, rest: &str, rest_at: usize, prefix: TextRange) -> Parsed<'_> {
    let Some(word) = next_word(rest) else {
        return Parsed::Malformed(Malformed {
            tool,
            kind: MalformedKind::MissingVerb,
            range: prefix,
            word: tool.prefix().to_string(),
        });
    };
    let Some(verb) = Verb::from_word(word) else {
        return Parsed::Malformed(Malformed {
            tool,
            kind: MalformedKind::UnknownVerb,
            range: TextRange::at(size(rest_at), size(word.len())),
            word: word.to_string(),
        });
    };

    let (tail, tail_at) = trim_start_at(&rest[word.len()..], rest_at + word.len());

    // A lint directive names its rule after the verb; `# arity` addresses the
    // linter with no way to name one, so its lint half is all-rules by
    // construction, and `# arity-format` has no lint half at all. Only the forms
    // with no rule slot can have a word here that means nothing.
    let takes_rule = tool == Tool::Lint && verb != Verb::On;
    if !takes_rule && let Some(extra) = next_word(tail) {
        return Parsed::Malformed(Malformed {
            tool,
            kind: MalformedKind::UnexpectedWord,
            range: TextRange::at(size(tail_at), size(extra.len())),
            word: extra.to_string(),
        });
    }
    let scope = if takes_rule {
        match parse_rule(tail, tail_at) {
            Some(rule) => RuleScope::Rule(rule),
            None if tail.starts_with(':') => RuleScope::All,
            None => RuleScope::Unnamed,
        }
    } else if tool == Tool::Both {
        RuleScope::All
    } else {
        RuleScope::Unnamed
    };
    Parsed::Directive(Directive {
        tool,
        verb,
        scope,
        reason: parse_reason(tail),
        spelling: Spelling::Canonical,
    })
}

/// Strip `word` from the head of `body` when it stands as a whole word — the
/// next character must end the word, so `arity-format` never matches inside
/// `arity-formatter`. Returns the trimmed remainder and its offset.
fn strip_word<'a>(body: &'a str, body_at: usize, word: &str) -> Option<(&'a str, usize)> {
    let rest = body.strip_prefix(word)?;
    match rest.chars().next() {
        None => Some((rest, body_at + word.len())),
        Some(c) if c.is_whitespace() || c == ':' => Some(trim_start_at(rest, body_at + word.len())),
        Some(_) => None,
    }
}

/// The first word of `rest`, which callers pass already trimmed so the word's
/// offset is the caller's own `rest_at`. A word runs to the first whitespace or
/// `:`; a `rest` that is empty or opens with `:` has none.
fn next_word(rest: &str) -> Option<&str> {
    let end = rest
        .find(|c: char| c == ':' || c.is_whitespace())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// The rule ID at the head of `rest`, if any.
fn parse_rule(rest: &str, rest_at: usize) -> Option<RuleRef> {
    let word = next_word(rest)?;
    Some(RuleRef {
        id: word.to_string(),
        range: TextRange::at(size(rest_at), size(word.len())),
    })
}

/// The reason is everything after the first `:`, trimmed.
fn parse_reason(rest: &str) -> Option<&str> {
    let (_, reason) = rest.split_once(':')?;
    let trimmed = reason.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// `str::trim_start`, carrying the byte offset along.
fn trim_start_at(s: &str, offset: usize) -> (&str, usize) {
    let trimmed = s.trim_start();
    (trimmed, offset + (s.len() - trimmed.len()))
}

fn size(n: usize) -> TextSize {
    TextSize::from(n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directive(text: &str) -> Directive<'_> {
        match parse(text) {
            Some(Parsed::Directive(d)) => d,
            other => panic!("expected a directive, got {other:?}"),
        }
    }

    fn malformed(text: &str) -> Malformed {
        match parse(text) {
            Some(Parsed::Malformed(m)) => m,
            other => panic!("expected a malformed directive, got {other:?}"),
        }
    }

    fn rule_id(text: &str) -> Option<String> {
        directive(text).rule().map(|r| r.id.clone())
    }

    #[test]
    fn format_column() {
        let d = directive("# arity-format skip: hand-aligned");
        assert_eq!((d.tool, d.verb), (Tool::Format, Verb::Skip));
        assert_eq!(d.reason, Some("hand-aligned"));
        assert_eq!(d.scope, RuleScope::Unnamed);
        assert!(!d.has_rule_slot());

        assert_eq!(
            (
                directive("# arity-format off").tool,
                directive("# arity-format off").verb
            ),
            (Tool::Format, Verb::Off)
        );
        assert_eq!(directive("# arity-format on").verb, Verb::On);
        assert_eq!(
            directive("# arity-format skip-file: generated").verb,
            Verb::SkipFile
        );
    }

    #[test]
    fn lint_column() {
        let d = directive("# arity-lint skip unused-binding: documented API");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::Skip));
        assert_eq!(d.rule().unwrap().id, "unused-binding");
        assert_eq!(d.reason, Some("documented API"));
        assert_eq!(d.spelling, Spelling::Canonical);

        let d = directive("# arity-lint skip-file unused-binding: codegen");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::SkipFile));
        assert_eq!(d.rule().unwrap().id, "unused-binding");

        let d = directive("# arity-lint skip-file: noisy");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::SkipFile));
        assert_eq!(d.scope, RuleScope::All);
        assert_eq!(d.reason, Some("noisy"));
    }

    #[test]
    fn the_deprecated_spellings_still_parse_and_are_tagged() {
        let d = directive("# arity-ignore unused-binding: documented API");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::Skip));
        assert_eq!(d.rule().unwrap().id, "unused-binding");
        assert_eq!(d.reason, Some("documented API"));
        assert_eq!(d.spelling, Spelling::Deprecated);

        let d = directive("# arity-ignore-file unused-binding: codegen");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::SkipFile));
        assert_eq!(d.rule().unwrap().id, "unused-binding");
        assert_eq!(d.spelling, Spelling::Deprecated);

        let d = directive("# arity-ignore-file: noisy");
        assert_eq!(d.scope, RuleScope::All);
        assert_eq!(d.spelling, Spelling::Deprecated);
    }

    #[test]
    fn lint_regions() {
        let d = directive("# arity-lint off unused-binding: generated block");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::Off));
        assert_eq!(d.rule().unwrap().id, "unused-binding");
        assert_eq!(d.reason, Some("generated block"));

        // A `:` in rule position is the blanket region; nothing at all is inert.
        assert_eq!(directive("# arity-lint off: noisy").scope, RuleScope::All);
        let d = directive("# arity-lint off");
        assert_eq!(d.verb, Verb::Off);
        assert_eq!(d.scope, RuleScope::Unnamed);

        assert_eq!(directive("# arity-lint on").verb, Verb::On);
    }

    #[test]
    fn both_column() {
        for (text, verb) in [
            ("# arity skip: leave it", Verb::Skip),
            ("# arity off", Verb::Off),
            ("# arity on", Verb::On),
            ("# arity skip-file: vendored", Verb::SkipFile),
        ] {
            let d = directive(text);
            assert_eq!((d.tool, d.verb), (Tool::Both, verb), "{text}");
            // The `# arity` form covers every lint rule by construction.
            assert_eq!(d.scope, RuleScope::All, "{text}");
        }
    }

    #[test]
    fn a_bare_arity_directive_needs_a_verb() {
        // Prose about arity is not a directive: a bare `arity` only counts when
        // the next word is one of the four verbs.
        assert_eq!(parse("# arity is great"), None);
        assert_eq!(parse("# arity formats this badly"), None);
        assert_eq!(parse("# arity"), None);
        assert_eq!(parse("# arity: see below"), None);
    }

    #[test]
    fn a_plain_comment_is_not_a_directive() {
        assert_eq!(parse("# TODO: fix this"), None);
        assert_eq!(parse("#"), None);
        assert_eq!(parse(""), None);
        // A longer word that merely starts with a prefix must not match.
        assert_eq!(parse("# arity-formatter is the crate"), None);
        assert_eq!(parse("# arity-ignored by everyone"), None);
    }

    #[test]
    fn an_unknown_verb_is_reported_not_dropped() {
        let m = malformed("# arity-format skipp: typo");
        assert_eq!((m.tool, m.kind), (Tool::Format, MalformedKind::UnknownVerb));
        assert_eq!(m.word, "skipp");
    }

    #[test]
    fn a_missing_verb_is_reported() {
        assert_eq!(malformed("# arity-format").kind, MalformedKind::MissingVerb);
        assert_eq!(malformed("# arity-lint").kind, MalformedKind::MissingVerb);
        assert_eq!(
            malformed("# arity-format: because").kind,
            MalformedKind::MissingVerb
        );
    }

    #[test]
    fn a_rule_id_on_a_format_directive_is_reported() {
        let m = malformed("# arity-format skip unused-binding: no");
        assert_eq!(m.kind, MalformedKind::UnexpectedWord);
        assert_eq!(m.word, "unused-binding");
        // The `both` form's lint half is all-rules by construction, so naming a
        // rule there is ambiguous too.
        assert_eq!(
            malformed("# arity skip unused-binding: no").kind,
            MalformedKind::UnexpectedWord
        );
        // Same seam catches prose written without a colon.
        assert_eq!(malformed("# arity-format off do not touch").word, "do");
    }

    #[test]
    fn a_skip_verb_after_arity_ignore_is_reported() {
        let m = malformed("# arity-ignore skip unused-binding: mixed spellings");
        assert_eq!(
            (m.tool, m.kind),
            (Tool::Lint, MalformedKind::UnexpectedVerb)
        );
        assert_eq!(m.word, "skip");
        assert_eq!(
            malformed("# arity-ignore skip-file unused-binding: x").kind,
            MalformedKind::UnexpectedVerb
        );
    }

    #[test]
    fn a_reason_is_optional_and_trimmed() {
        assert_eq!(directive("# arity-format skip").reason, None);
        assert_eq!(directive("# arity-format skip:").reason, None);
        assert_eq!(directive("# arity-format skip:    ").reason, None);
        assert_eq!(
            directive("# arity-format skip:  spaced  ").reason,
            Some("spaced")
        );
        assert_eq!(directive("# arity-lint skip browser").reason, None);
    }

    #[test]
    fn a_directive_with_no_rule_named_is_inert() {
        // Shipped behavior: inert, and reported by `blanket-suppression`.
        let d = directive("# arity-lint skip");
        assert_eq!((d.tool, d.verb), (Tool::Lint, Verb::Skip));
        assert_eq!(d.scope, RuleScope::Unnamed);
        assert!(d.has_rule_slot());

        // Bare, it is inert; with a `:` the author asked for every rule.
        assert_eq!(
            directive("# arity-lint skip-file").scope,
            RuleScope::Unnamed
        );
        assert_eq!(
            directive("# arity-lint skip: because").scope,
            RuleScope::All
        );
        // And the same for the deprecated spellings.
        assert_eq!(directive("# arity-ignore").scope, RuleScope::Unnamed);
        assert_eq!(directive("# arity-ignore-file").scope, RuleScope::Unnamed);
    }

    #[test]
    fn an_on_names_nothing_by_design() {
        let d = directive("# arity-lint on");
        assert_eq!(d.scope, RuleScope::Unnamed);
        assert!(!d.has_rule_slot(), "an `on` closes whatever is open");
    }

    #[test]
    fn a_comma_list_reads_as_one_rule_id() {
        // Shipped behavior: the ID runs to the first whitespace, so this names
        // `browser,` and suppresses neither rule. `misnamed-suppression` says so.
        assert_eq!(
            rule_id("# arity-lint skip browser, repeat: debugging").as_deref(),
            Some("browser,")
        );
    }

    #[test]
    fn ranges_are_relative_to_the_comment_start() {
        let text = "#   arity-lint  skip   unused-binding: why";
        let d = directive(text);
        let rule = d.rule().unwrap();
        let start: usize = rule.range.start().into();
        let end: usize = rule.range.end().into();
        assert_eq!(&text[start..end], "unused-binding");

        let m = malformed("#  arity-format  skipp: typo");
        let start: usize = m.range.start().into();
        let end: usize = m.range.end().into();
        assert_eq!(&"#  arity-format  skipp: typo"[start..end], "skipp");
    }

    #[test]
    fn a_missing_verb_points_at_the_prefix() {
        let m = malformed("# arity-format");
        let start: usize = m.range.start().into();
        let end: usize = m.range.end().into();
        assert_eq!(&"# arity-format"[start..end], "arity-format");
        assert_eq!(m.word, "arity-format");
    }

    #[test]
    fn tool_reports_what_it_affects() {
        assert!(Tool::Both.affects_lint() && Tool::Both.affects_format());
        assert!(Tool::Format.affects_format() && !Tool::Format.affects_lint());
        assert!(Tool::Lint.affects_lint() && !Tool::Lint.affects_format());
    }
}
