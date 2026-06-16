use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use lsp_types::Uri;

/// Convert a `file:` URI to a filesystem path, or `None` if it isn't a file
/// URI or has no scheme.
pub fn to_path(uri: &Uri) -> Option<PathBuf> {
    let scheme = uri.scheme()?;
    if !scheme.as_str().eq_ignore_ascii_case("file") {
        return None;
    }
    let decoded = uri
        .path()
        .as_estr()
        .decode()
        .into_string_lossy()
        .into_owned();
    Some(from_uri_path(&decoded))
}

#[cfg(windows)]
fn from_uri_path(p: &str) -> PathBuf {
    // "/C:/Users/x" → "C:\Users\x"
    PathBuf::from(p.strip_prefix('/').unwrap_or(p).replace('/', "\\"))
}

#[cfg(not(windows))]
fn from_uri_path(p: &str) -> PathBuf {
    PathBuf::from(p)
}

/// Convert a filesystem path to a `file:` URI. Used by go-to-definition to
/// name a cross-file target (the client always supplies URIs in real traffic,
/// so request handling never needs this) and by the tests.
pub fn from_path(path: &Path) -> Option<Uri> {
    let s = path.to_str()?;
    let mut out = String::from("file://");
    encode_into(&to_uri_path(s), &mut out);
    Uri::from_str(&out).ok()
}

#[cfg(windows)]
fn to_uri_path(s: &str) -> String {
    // "C:\Users\x" → "/C:/Users/x" (the URI path needs a leading slash)
    format!("/{}", s.replace('\\', "/"))
}

#[cfg(not(windows))]
fn to_uri_path(s: &str) -> String {
    s.to_string()
}

/// Percent-encode `s`, leaving the unreserved set plus `/` and `:` (drive
/// letters) intact.
fn encode_into(s: &str, out: &mut String) {
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex(b >> 4));
            out.push(hex(b & 0x0f));
        }
    }
}

fn hex(n: u8) -> char {
    char::from(if n < 10 { b'0' + n } else { b'A' + (n - 10) })
}

#[cfg(test)]
mod tests {
    use crate::lsp::test_support::*;
    use crate::lsp::uri;

    #[test]
    fn uri_path_round_trips() {
        let uri = test_uri();
        assert_eq!(uri::to_path(&uri).as_deref(), Some(test_path()));
    }
}
