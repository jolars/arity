//! Statically resolving an `Authors@R` value into the people it names.
//!
//! `Authors@R` is R code, and R *evaluates* it — `.read_authors_at_R_field`
//! runs `str2expression` and then `eval`. arity does not evaluate anything
//! (the static-semantics tenet), so this reads the same source with arity's own
//! R parser and resolves only what is decidable from the text: literal strings,
//! `c(...)` of literal strings, and the `person()` calls they are arguments to.
//! Exactly the trick `src/project/description.rs` plays on the `Roxygen` field.
//!
//! Anything computed — a variable, a `paste()`, an argument arity cannot see
//! through — resolves to [`Value::Unknown`] and every finding that depends on
//! it is withheld. That is the whole safety argument: the resolver never
//! guesses what R would have produced, so the rule cannot report a defect a
//! package does not have.

use std::sync::LazyLock;

use rowan::TextRange;
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, CallExpr, HasArgList as _};
use crate::linter::rules::matchers::string_literal;
use crate::linter::rules::packaging::scalar_field::Folded;
use crate::parser::parse;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// The calls `.read_authors_at_R_field(strict = TRUE)` will let through. It is
/// about to evaluate the field, so the list is short and the check is R's, not
/// arity's opinion about which functions are tasteful.
///
/// `(` is R's own entry for a parenthesized expression and has no callee in
/// arity's CST, so it is handled structurally rather than named here.
const SAFE_CALLS: &[&str] = &["person", "as.person", "c", "list", "paste", "paste0"];

/// `person()`'s formals, in the order R matches positional arguments against.
const FORMALS: &[&str] = &[
    "given", "family", "middle", "email", "role", "comment", "first", "last",
];

/// The MARC relator table `person()` canonicalizes roles against, as
/// `code<TAB>term` lines. Generated from R:
///
/// ```sh
/// Rscript -e 'db <- utils:::MARC_relator_db
///   cat(paste(db$code, db$term, sep = "\t"), sep = "\n")'
/// ```
const RELATORS: &str = include_str!("relators.txt");

static RELATOR_CODES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    RELATORS
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(code, _)| code)
        .collect()
});

static RELATOR_TERMS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    RELATORS
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(_, term)| term)
        .collect()
});

/// What the field turned out to be.
pub enum Authors {
    /// Not R at all. R's `str2expression` raises this before anything else.
    Unparseable,
    /// A call R's strict reader refuses, and the source range naming it.
    UnsafeCall(String, TextRange),
    /// The people the field names, resolved.
    Persons(Resolved),
    /// R, and safe, but not statically resolvable — a bare variable, an
    /// `as.person` over one, a value built by `paste()`.
    Unresolved,
}

/// What a resolvable `Authors@R` value came to.
pub struct Resolved {
    /// The people R ends up with — what every question about authorship and
    /// maintainership is asked of.
    pub persons: Vec<Person>,
    /// The `person()` calls supplying nothing at all. R returns a
    /// **zero-length** person vector for those, so they are not nameless
    /// people; they are not people, and no clause about authorship reaches
    /// them. `description-empty-person` is their subject.
    pub empty: Vec<TextRange>,
}

/// One `person(...)` call, with each argument resolved as far as the text goes.
pub struct Person {
    /// The call's source range — what a finding about this person spans.
    pub range: TextRange,
    pub given: Value,
    pub family: Value,
    pub email: Value,
    pub role: Value,
    pub comment: Value,
}

/// An argument's value, as far as it is decidable without running R.
pub enum Value {
    /// Not supplied, or `NULL`/`NA`, which R's `person()` canonicalizes to the
    /// same absence.
    Missing,
    /// Supplied, but computed: the resolver has nothing to say about it.
    Unknown,
    /// Literal strings, in written order.
    Strings(Vec<Literal>),
}

/// One string literal inside a resolved value, with the name it was written
/// under (`c(ORCID = "…")`) and the source range to span.
pub struct Literal {
    pub name: Option<String>,
    pub text: String,
    pub range: TextRange,
}

impl Value {
    /// Whether the value holds text R would keep, or `None` when that is not
    /// decidable. `person()` trims and drops empty strings, so a whitespace-only
    /// name is no name.
    pub fn is_present(&self) -> Option<bool> {
        match self {
            Self::Missing => Some(false),
            Self::Unknown => None,
            Self::Strings(items) => Some(items.iter().any(|s| !s.text.trim().is_empty())),
        }
    }

    /// The literals, or an empty slice when there are none to look at.
    pub fn literals(&self) -> &[Literal] {
        match self {
            Self::Strings(items) => items,
            _ => &[],
        }
    }

    /// The first literal written under `name`, which is how R reads
    /// `comment["ORCID"]`.
    pub fn named(&self, name: &str) -> Option<&Literal> {
        self.literals()
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
    }
}

impl Person {
    /// Whether R definitely materializes this person: at least one argument
    /// resolves to text, which is what makes the call more than
    /// [`is_empty`](Self::is_empty) could ever be.
    ///
    /// A person built entirely out of computed arguments could still be
    /// `person(given = NULL)` — R's zero-length vector — so nothing structural
    /// is said about them.
    pub fn is_materialized(&self) -> bool {
        [
            &self.given,
            &self.family,
            &self.email,
            &self.role,
            &self.comment,
        ]
        .iter()
        .any(|value| !value.literals().is_empty())
    }

    /// Whether the call supplies nothing at all. `person()` — and
    /// `person(NULL)` — returns a **zero-length** person vector, so it names
    /// nobody rather than naming a nameless somebody. A real one turns up at
    /// the end of a hand-maintained `c(...)`.
    fn is_empty(&self) -> bool {
        [
            &self.given,
            &self.family,
            &self.email,
            &self.role,
            &self.comment,
        ]
        .iter()
        .all(|value| matches!(value, Value::Missing))
    }

    /// Whether R would find a usable name — `given` or `family`. `None` when
    /// either is computed and the answer could go both ways.
    pub fn has_name(&self) -> Option<bool> {
        match (self.given.is_present(), self.family.is_present()) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }
    }

    /// Whether the person carries `code` as a role. `None` when a role is
    /// computed, or when one is written in a spelling R would canonicalize —
    /// either could turn into `code` and neither is arity's to decide.
    pub fn has_role(&self, code: &str) -> Option<bool> {
        match &self.role {
            Value::Missing => Some(false),
            Value::Unknown => None,
            Value::Strings(items) => {
                if items.iter().any(|item| item.text.trim() == code) {
                    Some(true)
                } else if items.iter().any(|item| !is_relator_code(item.text.trim())) {
                    None
                } else {
                    Some(false)
                }
            }
        }
    }
}

/// Read the folded `Authors@R` value, mapping every range back into the
/// `DESCRIPTION`.
pub fn resolve(folded: &Folded) -> Authors {
    let output = parse(&folded.text);
    if !output.diagnostics.is_empty() {
        return Authors::Unparseable;
    }

    if let Some((name, range)) = unsafe_call(&output.cst) {
        return Authors::UnsafeCall(name.to_string(), folded.map(range));
    }

    // R evaluates the whole expression list and takes the last value, so the
    // last top-level expression is the one that names the people. Walk elements
    // rather than nodes: a bare symbol statement is a token in this CST.
    let Some(last) = output
        .cst
        .children_with_tokens()
        .filter(|el| !is_ignorable(el.kind()))
        .last()
    else {
        return Authors::Unresolved;
    };

    match resolve_persons(&last, folded) {
        Some(resolved) => Authors::Persons(resolved),
        None => Authors::Unresolved,
    }
}

/// Whether the trimmed value of `Author` is R code written under the wrong key,
/// mirroring CRAN's `author_should_be_authors_at_R` regexp
/// `^(Authors@R *:|person *\(|c *\()`.
pub fn looks_like_r_code(author: &str) -> bool {
    let author = author.trim_start();
    for (head, tail) in [("Authors@R", ":"), ("person", "("), ("c", "(")] {
        let Some(rest) = author.strip_prefix(head) else {
            continue;
        };
        if rest.trim_start_matches(' ').starts_with(tail) {
            return true;
        }
    }
    false
}

/// CRAN's `author_starts_with_Author`: `^Author *:`.
pub fn starts_with_its_own_field_name(author: &str) -> bool {
    author
        .trim_start()
        .strip_prefix("Author")
        .is_some_and(|rest| rest.trim_start_matches(' ').starts_with(':'))
}

/// Whether `role` is one `person()` keeps as written.
///
/// A role that is not a relator code is passed to `.canonicalize_person_role`,
/// which partial-matches it against the relator *terms* and drops it when that
/// fails. arity only reports the roles that fail both, so a spelling R's
/// fallback resolves is left alone rather than reported on arity's authority.
pub fn is_known_role(role: &str) -> bool {
    let role = role.trim();
    if is_relator_code(role) {
        return true;
    }
    let lower = role.to_lowercase();
    !lower.is_empty() && RELATOR_TERMS.iter().any(|term| term.starts_with(&lower))
}

fn is_relator_code(role: &str) -> bool {
    RELATOR_CODES.contains(&role)
}

/// R's `.ORCID_iD_is_valid`: one of the written variants, then the MOD 11-2
/// check digit. Self-validating, so no network is involved.
pub fn is_valid_orcid(id: &str) -> bool {
    let Some(core) = orcid_core(id) else {
        return false;
    };
    let digits: Vec<char> = core.chars().filter(|c| *c != '-').collect();
    let total: u32 = digits[..15]
        .iter()
        .enumerate()
        .map(|(i, c)| c.to_digit(10).unwrap_or(0) << (15 - i))
        .sum();
    let check = (12 - total % 11) % 11;
    let expected = if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10).unwrap_or('X')
    };
    digits[15] == expected
}

/// R's `.ORCID_iD_variants_regexp`, less the check digit: an optional `<…>`, an
/// optional `orcid.org/` with an optional scheme, then `dddd-dddd-dddd-dddC`.
fn orcid_core(id: &str) -> Option<&str> {
    let id = id.strip_prefix('<').unwrap_or(id);
    let id = id.strip_suffix('>').unwrap_or(id);
    let id = match id
        .strip_prefix("https://")
        .or_else(|| id.strip_prefix("http://"))
    {
        Some(rest) => rest.strip_prefix("orcid.org/")?,
        None => id.strip_prefix("orcid.org/").unwrap_or(id),
    };
    is_orcid_shape(id).then_some(id)
}

/// R's `.ORCID_iD_regexp`: `[0-9]{4}-[0-9]{4}-[0-9]{4}-[0-9]{3}[X0-9]`.
fn is_orcid_shape(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    if groups.len() != 4 || groups.iter().any(|g| g.len() != 4) {
        return false;
    }
    let head = groups[..3]
        .iter()
        .all(|g| g.bytes().all(|b| b.is_ascii_digit()));
    let last = groups[3].as_bytes();
    head && last[..3].iter().all(u8::is_ascii_digit)
        && (last[3] == b'X' || last[3].is_ascii_digit())
}

/// Whether an unnamed `comment` entry is one `person()` labels `ORCID` itself:
/// `^https?://orcid.org/<id>$`.
pub fn is_orcid_url(text: &str) -> bool {
    text.strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .and_then(|rest| rest.strip_prefix("orcid.org/"))
        .is_some_and(is_orcid_shape)
}

/// R's `.ROR_ID_is_valid`, which is its variants regexp and nothing else:
/// `^<?((https://|)ror.org/)?(.{9})>?$`. A ROR ID carries no check digit, so
/// nine characters is the whole of what is decidable offline.
pub fn is_valid_ror(id: &str) -> bool {
    let id = id.strip_prefix('<').unwrap_or(id);
    let id = id.strip_suffix('>').unwrap_or(id);
    let id = match id.strip_prefix("https://") {
        Some(rest) => match rest.strip_prefix("ror.org/") {
            Some(rest) => rest,
            None => return false,
        },
        None => id.strip_prefix("ror.org/").unwrap_or(id),
    };
    id.chars().count() == 9
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// The first call R's strict reader would refuse, as `(name, range)`.
///
/// A `::`/`:::` is itself one of those calls — R names both `utils::person(…)`
/// and `utils::person` — so its presence is reported over the callee it wraps,
/// which arity's CST reads as a plain `person`.
fn unsafe_call(root: &SyntaxNode) -> Option<(SmolStr, TextRange)> {
    for el in root.descendants_with_tokens() {
        match el {
            SyntaxElement::Token(token)
                if matches!(token.kind(), SyntaxKind::COLON2 | SyntaxKind::COLON3) =>
            {
                let expr = token.parent()?;
                return Some((SmolStr::new(expr.text().to_string()), expr.text_range()));
            }
            SyntaxElement::Node(node) if node.kind() == SyntaxKind::CALL_EXPR => {
                let call = CallExpr::cast(node)?;
                let name = call.callee_name()?;
                if !SAFE_CALLS.contains(&name.as_str()) {
                    let range = call
                        .callee_token()
                        .map_or_else(|| call.syntax().text_range(), |t| t.text_range());
                    return Some((name, range));
                }
            }
            _ => {}
        }
    }
    None
}

/// The people an expression names, or `None` when it is not resolvable.
fn resolve_persons(el: &SyntaxElement, folded: &Folded) -> Option<Resolved> {
    let node = el.as_node()?;
    match node.kind() {
        SyntaxKind::PAREN_EXPR => {
            let inner = node
                .children_with_tokens()
                .filter(|e| !is_ignorable(e.kind()) && !is_paren(e.kind()))
                .last()?;
            resolve_persons(&inner, folded)
        }
        SyntaxKind::CALL_EXPR => {
            let call = CallExpr::cast(node.clone())?;
            match call.callee_name()?.as_str() {
                "person" => {
                    let resolved = person(&call, folded);
                    Some(if resolved.is_empty() {
                        Resolved {
                            persons: Vec::new(),
                            empty: vec![resolved.range],
                        }
                    } else {
                        Resolved {
                            persons: vec![resolved],
                            empty: Vec::new(),
                        }
                    })
                }
                "c" | "list" => {
                    let mut all = Resolved {
                        persons: Vec::new(),
                        empty: Vec::new(),
                    };
                    for arg in call.args() {
                        let inner = resolve_persons(&arg.value()?, folded)?;
                        all.persons.extend(inner.persons);
                        all.empty.extend(inner.empty);
                    }
                    Some(all)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// One `person(...)` call, with its arguments matched to R's formals.
fn person(call: &CallExpr, folded: &Folded) -> Person {
    let mut slots: Vec<Option<Value>> = (0..FORMALS.len()).map(|_| None).collect();
    let mut positional: Vec<Arg> = Vec::new();

    for arg in call.args() {
        match arg.name().as_deref().and_then(match_formal) {
            Some(index) if slots[index].is_none() => slots[index] = Some(value(&arg, folded)),
            // A name arity cannot match to a formal is one R would error on,
            // so nothing here is decidable any more.
            Some(_) => {}
            None if arg.is_named() => slots.iter_mut().for_each(|slot| {
                if slot.is_none() {
                    *slot = Some(Value::Unknown);
                }
            }),
            None => positional.push(arg),
        }
    }

    let mut positional = positional.into_iter();
    for slot in slots.iter_mut() {
        if slot.is_some() {
            continue;
        }
        let Some(arg) = positional.next() else {
            break;
        };
        *slot = Some(value(&arg, folded));
    }

    let mut take = |name: &str| {
        let index = FORMALS.iter().position(|f| *f == name).expect("a formal");
        slots[index].take().unwrap_or(Value::Missing)
    };
    Person {
        range: folded.map(call.syntax().text_range()),
        given: take("given"),
        family: take("family"),
        email: take("email"),
        role: take("role"),
        comment: take("comment"),
    }
}

/// R's argument matching: an exact formal name, else a unique prefix of one.
fn match_formal(name: &str) -> Option<usize> {
    if let Some(exact) = FORMALS.iter().position(|f| *f == name) {
        return Some(exact);
    }
    let mut matches = FORMALS
        .iter()
        .enumerate()
        .filter(|(_, formal)| formal.starts_with(name));
    let only = matches.next()?;
    matches.next().is_none().then_some(only.0)
}

/// An argument's value, resolved as far as the text goes.
fn value(arg: &Arg, folded: &Folded) -> Value {
    // A value-less argument is R's empty slot (`person("A", "B", , "a@b.c")`),
    // which is the formal's default.
    let Some(el) = arg.value() else {
        return Value::Missing;
    };
    element_value(&el, folded)
}

fn element_value(el: &SyntaxElement, folded: &Folded) -> Value {
    match el {
        SyntaxElement::Token(token) => match token.kind() {
            SyntaxKind::STRING => match string_literal(token).map(|(_, inner)| inner) {
                // An escape would have to be unescaped to be compared, and
                // this resolver does not own R's escape table.
                Some(inner) if !inner.contains('\\') => Value::Strings(vec![Literal {
                    name: None,
                    text: inner.to_string(),
                    range: folded.map(token.text_range()),
                }]),
                _ => Value::Unknown,
            },
            SyntaxKind::IDENT if matches!(token.text(), "NULL" | "NA" | "NA_character_") => {
                Value::Missing
            }
            _ => Value::Unknown,
        },
        SyntaxElement::Node(node) => match node.kind() {
            SyntaxKind::PAREN_EXPR => node
                .children_with_tokens()
                .filter(|e| !is_ignorable(e.kind()) && !is_paren(e.kind()))
                .last()
                .map_or(Value::Unknown, |inner| element_value(&inner, folded)),
            SyntaxKind::CALL_EXPR => concatenation(node, folded),
            _ => Value::Unknown,
        },
    }
}

/// A `c(...)` of literal strings, which is how every real `role` and `comment`
/// is written. Anything else in it makes the whole value unknown.
fn concatenation(node: &SyntaxNode, folded: &Folded) -> Value {
    let Some(call) = CallExpr::cast(node.clone()) else {
        return Value::Unknown;
    };
    if call.callee_name().as_deref() != Some("c") {
        return Value::Unknown;
    }
    let mut items = Vec::new();
    for arg in call.args() {
        let Some(el) = arg.value() else {
            continue;
        };
        match element_value(&el, folded) {
            Value::Missing => {}
            Value::Unknown => return Value::Unknown,
            Value::Strings(inner) => {
                let name = arg.name().map(|n| n.to_string());
                for mut item in inner {
                    // `c(ORCID = "…")` names its one element; a name over a
                    // nested vector is R's `ORCID1`/`ORCID2` spelling, which
                    // nothing here reads.
                    if item.name.is_none() {
                        item.name = name.clone();
                    }
                    items.push(item);
                }
            }
        }
    }
    if items.is_empty() {
        Value::Missing
    } else {
        Value::Strings(items)
    }
}

fn is_ignorable(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT | SyntaxKind::SEMICOLON
    )
}

fn is_paren(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::LPAREN | SyntaxKind::RPAREN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identifier in `?person`, and R's own example. Verified against
    /// `tools:::.ORCID_iD_is_valid`.
    #[test]
    fn orcid_check_digits_match_r() {
        assert!(is_valid_orcid("0000-0002-1825-0097"));
        assert!(is_valid_orcid("https://orcid.org/0000-0002-1825-0097"));
        assert!(is_valid_orcid("<orcid.org/0000-0002-1825-0097>"));
        assert!(!is_valid_orcid("1234-5678-9012-3456"));
        assert!(!is_valid_orcid("0000-0002-1825-0098"));
        assert!(!is_valid_orcid("0000-0002-1825"));
        assert!(!is_valid_orcid("https://example.com/0000-0002-1825-0097"));
    }

    /// The `X` check digit is a real ORCID spelling, not a placeholder.
    #[test]
    fn an_x_check_digit_is_valid() {
        assert!(is_valid_orcid("0000-0002-1694-233X"));
    }

    /// R's ROR check is its variants regexp and nothing more.
    #[test]
    fn ror_ids_are_nine_characters() {
        assert!(is_valid_ror("03wc8by49"));
        assert!(is_valid_ror("https://ror.org/03wc8by49"));
        assert!(!is_valid_ror("12345"));
        assert!(!is_valid_ror("03wc8by490"));
    }

    #[test]
    fn relator_codes_and_terms_are_both_read() {
        assert!(is_known_role("aut"));
        assert!(is_known_role("spy")); // a real MARC code, however it reads
        assert!(is_known_role("compiler")); // a term R's fallback resolves
        assert!(!is_known_role("zzz"));
        assert!(!is_known_role(""));
    }
}
