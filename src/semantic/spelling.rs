//! Conservative spelling candidates from statically known names.

use std::collections::BTreeSet;

use smol_str::SmolStr;

pub(crate) const MAX_NAMES: usize = 8192;
pub(crate) const MAX_CHARS: usize = 64;

/// Refuse a truncated pool: an unseen name could make a suggestion ambiguous.
pub(crate) fn candidate_names<'a>(
    names: impl Iterator<Item = &'a SmolStr>,
) -> Option<BTreeSet<SmolStr>> {
    let mut out = BTreeSet::new();
    for (index, name) in names.enumerate() {
        if index == MAX_NAMES {
            return None;
        }
        if bounded_chars(name).is_some() {
            out.insert(name.clone());
        }
    }
    Some(out)
}

pub(crate) fn bounded_chars(name: &str) -> Option<Vec<char>> {
    if name.len() > MAX_CHARS * 4 {
        return None;
    }
    let chars: Vec<_> = name.chars().take(MAX_CHARS + 1).collect();
    (chars.len() <= MAX_CHARS).then_some(chars)
}

/// One insertion, deletion, substitution, or adjacent transposition. Compare
/// Unicode scalar values so a multibyte character counts as one spelling edit.
pub(crate) fn one_edit(a: &[char], b: &[char]) -> bool {
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let prefix = a.iter().zip(b).take_while(|(a, b)| a == b).count();
    let a = &a[prefix..];
    let b = &b[prefix..];
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Less => a == &b[1..],
        std::cmp::Ordering::Greater => &a[1..] == b,
        std::cmp::Ordering::Equal => {
            !a.is_empty()
                && (a[1..] == b[1..]
                    || (a.len() >= 2 && a[0] == b[1] && a[1] == b[0] && a[2..] == b[2..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_one_spelling_edit() {
        for (a, b, expected) in [
            ("lenth", "length", true),
            ("lenght", "length", true),
            ("lengthh", "length", true),
            ("lenggh", "length", true),
            ("length", "length", false),
            ("lenhg", "length", false),
            ("résultt", "résultat", true),
            ("", "a", true),
            ("", "", false),
        ] {
            let a: Vec<_> = a.chars().collect();
            let b: Vec<_> = b.chars().collect();
            assert_eq!(one_edit(&a, &b), expected);
            assert_eq!(one_edit(&b, &a), expected);
        }
    }

    #[test]
    fn refuses_truncated_candidate_pools() {
        let name = SmolStr::new("customer_total");
        assert!(candidate_names(std::iter::repeat_n(&name, MAX_NAMES)).is_some());
        assert!(candidate_names(std::iter::repeat_n(&name, MAX_NAMES + 1)).is_none());
        assert!(bounded_chars(&"x".repeat(MAX_CHARS + 1)).is_none());
        assert_eq!(
            bounded_chars(&"é".repeat(MAX_CHARS)).unwrap().len(),
            MAX_CHARS
        );
    }
}
