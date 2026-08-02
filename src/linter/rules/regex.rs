//! §I2: shared regex/string-literal pattern classification for the base-R regex
//! rules (`fixed-regex`, `string-boundary`).
//!
//! The rules read a `STRING` token's unquoted inner text with
//! [`matchers::string_literal`](super::matchers::string_literal); this module
//! classifies that text: whether it is a plain fixed string (no regex
//! metacharacter) and whether it is anchored at exactly one end (`^…`/`…$`).

/// Whether `s` contains no regex metacharacter (and no backslash escape), so it
/// matches literally and identically under both regex and `fixed = TRUE`
/// semantics. All metacharacters are ASCII, so a byte scan is sufficient.
pub fn is_plain_literal(s: &str) -> bool {
    !s.bytes().any(|b| {
        matches!(
            b,
            b'.' | b'\\'
                | b'|'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'^'
                | b'$'
                | b'*'
                | b'+'
                | b'?'
        )
    })
}

/// A non-empty plain literal: the "this pattern is really a fixed string" test
/// both regex rules share (an empty pattern is never a rewrite candidate).
pub fn is_fixed_string(s: &str) -> bool {
    !s.is_empty() && is_plain_literal(s)
}

/// Which end a pattern is anchored to.
pub enum Anchor {
    Start,
    End,
}

/// If `inner` is anchored at exactly one end against a non-empty plain literal,
/// the anchor and the anchor-stripped remainder: `^abc` -> `(Start, "abc")`,
/// `abc$` -> `(End, "abc")`. `None` for a both-ends `^abc$` (an exact match, not
/// a boundary test), no anchor, an empty remainder, or a remainder carrying any
/// other regex metacharacter.
pub fn single_anchor(inner: &str) -> Option<(Anchor, &str)> {
    let (anchor, rest) = if let Some(rest) = inner.strip_prefix('^') {
        // `^abc$` is a both-ends anchor (an exact match), not a boundary test.
        if rest.ends_with('$') {
            return None;
        }
        (Anchor::Start, rest)
    } else {
        // Reached only when `inner` does not start with `^`, so this is a lone
        // trailing anchor.
        (Anchor::End, inner.strip_suffix('$')?)
    };
    is_fixed_string(rest).then_some((anchor, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_literal_rejects_metacharacters() {
        assert!(is_plain_literal("abc"));
        assert!(is_plain_literal("hello world"));
        assert!(!is_plain_literal("a.b"));
        assert!(!is_plain_literal("a\\.b"));
        assert!(!is_plain_literal("^abc"));
        assert!(!is_plain_literal("a+b"));
    }

    #[test]
    fn fixed_string_requires_nonempty_plain() {
        assert!(is_fixed_string("abc"));
        assert!(!is_fixed_string(""));
        assert!(!is_fixed_string("a.b"));
    }

    #[test]
    fn single_anchor_classifies_one_end() {
        assert!(matches!(
            single_anchor("^abc"),
            Some((Anchor::Start, "abc"))
        ));
        assert!(matches!(single_anchor("abc$"), Some((Anchor::End, "abc"))));
        // Both-ends anchor is an exact match, not a boundary test.
        assert!(single_anchor("^abc$").is_none());
        // No anchor at all.
        assert!(single_anchor("abc").is_none());
        // Empty remainder after stripping the sole anchor.
        assert!(single_anchor("^").is_none());
        assert!(single_anchor("$").is_none());
        // Remainder carries another metacharacter.
        assert!(single_anchor("^a.b").is_none());
    }
}
