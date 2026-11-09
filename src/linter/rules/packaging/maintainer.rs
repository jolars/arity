//! R's maintainer field regexp, shared by explicit and derived maintainers.

/// R's second alternative: the package has no maintainer, spelled in capitals.
pub(super) const ORPHANED: &str = "ORPHANED";

/// The local-part character class of R's regexp, less the alphanumerics.
const LOCAL_SPECIALS: &str = "!#$%*/?|^{}`~&'+=_-";

/// R's `^[[:space:]]*(.*<LOCAL@DOMAIN>|ORPHANED)[[:space:]]*$`, for a trimmed value.
pub(super) fn is_valid_maintainer_field(maintainer: &str) -> bool {
    if maintainer == ORPHANED {
        return true;
    }
    let Some(inner) = maintainer.strip_suffix('>') else {
        return false;
    };
    // R backtracks through the greedy name prefix: a quoted local part can
    // contain `<`, so the last opening bracket need not start the address.
    inner
        .match_indices('<')
        .rev()
        .any(|(open, _)| is_valid_address(&inner[open + 1..]))
}

/// R's `LOCAL@DOMAIN`. The domain half admits no `@`, so the separator is the
/// last one — which is also what lets a quoted local part contain one.
fn is_valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    is_valid_local_part(local) && is_valid_domain(domain)
}

/// R's `(\".+\"|(ATOM+\.)*ATOM+)`. The quoted form is a single `.+` between
/// quotes, so it takes any characters at all — including a `\"` or an `@`.
fn is_valid_local_part(local: &str) -> bool {
    if local.len() > 2 && local.starts_with('"') && local.ends_with('"') {
        return true;
    }
    local
        .split('.')
        .all(|atom| !atom.is_empty() && atom.chars().all(is_local_char))
}

/// R's `([ABC...z0-9!#$%*/?|^{}`~&'+=_-]`, spelled out as ASCII in the regexp
/// and so read as ASCII here — the name half is `.*` and takes any text, but the
/// address is not.
fn is_local_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || LOCAL_SPECIALS.contains(c)
}

/// R's `([A-Za-z0-9-]+\.)*[A-Za-z0-9-]+`: no empty label, no TLD requirement,
/// and a leading `-` in a label is allowed.
fn is_valid_domain(domain: &str) -> bool {
    domain.split('.').all(|label| {
        !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}
