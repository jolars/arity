//! Native routines a package binds through `useDynLib()`.
//!
//! `useDynLib(pkg, .registration = TRUE)` binds every routine of the package's
//! registration table in its namespace, so R code may reference one anywhere a
//! value goes — passed along (`capture_arg = ffi_enquo`), compared
//! (`identical(x, ffi_enquo)`), not only in a `.Call()` head. Those names exist
//! in the C sources and nowhere in the package's R sources, so without this
//! module every such reference is an `undefined-symbol` false positive.
//!
//! The table is *harvested*, not assumed: reading `src/` gives the exact set,
//! whereas suppressing every unresolved name in a package declaring
//! `.registration = TRUE` would silence the rule across the whole package. The
//! price is that a registration shape this scanner does not recognize leaves the
//! false positives in place — the conservative direction is the one that keeps
//! reporting.
//!
//! No C is compiled or preprocessed here, and none is needed: the entry name is
//! a literal in the table, either as a string or as the first argument of the
//! stringifying macro (`#define CALLDEF(name, n) {#name, ...}`) that packages
//! write it with.

use std::collections::BTreeSet;
use std::path::Path;

use crate::rindex::harvest::parse_namespace;

/// The four `R_registerRoutines` table types. Their entries share one shape, so
/// which interface a routine belongs to doesn't matter: it is the R-visible name
/// this module is after.
const TABLE_TYPES: [&str; 4] = [
    "R_CallMethodDef",
    "R_CMethodDef",
    "R_FortranMethodDef",
    "R_ExternalMethodDef",
];

/// Source extensions worth scanning for a registration table.
const C_SOURCE_EXTS: [&str; 8] = ["c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx"];

/// How deep under `src/` to look. rlang keeps its table in
/// `src/internal/internal.c`, so the top level alone is not enough.
const MAX_DEPTH: usize = 4;

/// A ceiling on the bytes one package's harvest reads, so a directory that
/// happens to hold a vendored C library costs a bounded amount rather than
/// however much is on disk.
const MAX_BYTES: usize = 32 * 1024 * 1024;

/// The names `namespace`'s `useDynLib()` directives bind in the package rooted
/// at `root`: the ones the NAMESPACE enumerates itself, plus — when it declares
/// `.registration = TRUE` — the registration table harvested from `root/src`,
/// each wrapped in the declared `.fixes`.
///
/// Touches disk (only under `.registration = TRUE`); the pure half is
/// [`registered_routines`].
pub fn dynlib_bound_names(namespace: &str, root: &Path) -> BTreeSet<String> {
    let info = parse_namespace(namespace, &[]);
    let mut names = info.dynlib_routines;
    if let Some(fixes) = info.dynlib_registration {
        names.extend(
            harvest_registered_routines(root)
                .iter()
                .map(|routine| fixes.apply(routine)),
        );
    }
    names
}

/// Every routine name registered by the C sources under `root/src`.
fn harvest_registered_routines(root: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut budget = MAX_BYTES;
    harvest_dir(&root.join("src"), MAX_DEPTH, &mut budget, &mut names);
    names
}

/// Recursive half of [`harvest_registered_routines`]. Unreadable directories and
/// files are skipped rather than reported: a package's `src/` is input, and
/// nothing here may fail a lint.
fn harvest_dir(dir: &Path, depth: usize, budget: &mut usize, names: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // Sorted so a hit budget truncates deterministically rather than by
    // whatever order the filesystem hands back. The entry's own file type
    // avoids a `stat` per name.
    let mut paths: Vec<_> = entries
        .flatten()
        .filter_map(|e| Some((e.file_type().ok()?.is_dir(), e.path())))
        .collect();
    paths.sort();
    for (is_dir, path) in paths {
        if is_dir {
            if depth > 0 {
                harvest_dir(&path, depth - 1, budget, names);
            }
            continue;
        }
        let is_source = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| C_SOURCE_EXTS.contains(&e.to_ascii_lowercase().as_str()));
        if !is_source {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.len() > *budget {
            return;
        }
        *budget -= text.len();
        names.extend(registered_routines(&text));
    }
}

/// The routine names one C translation unit registers: for each
/// `R_CallMethodDef`-family table, the R-visible name of every entry.
///
/// Pure over the source text, and deliberately forgiving — an initializer shape
/// it can't read yields no name rather than a wrong one.
pub fn registered_routines(source: &str) -> BTreeSet<String> {
    let text = strip_comments(source);
    let mut names = BTreeSet::new();
    for table in TABLE_TYPES {
        let mut from = 0;
        while let Some(rel) = text[from..].find(table) {
            let start = from + rel;
            from = start + table.len();
            if !is_whole_word(&text, start, table.len()) {
                continue;
            }
            let Some(open) = initializer_brace(&text, from) else {
                continue;
            };
            let Some(close) = matching_brace(&text, open) else {
                continue;
            };
            names.extend(table_entry_names(&text[open + 1..close]));
            from = close;
        }
    }
    names
}

/// The offset of the `{` opening a table's initializer, scanning from just after
/// the type name. Only a declarator may intervene (`CallEntries[] =`), so
/// anything else — most importantly a `;` ending a forward declaration or a `(`
/// starting a parameter list — means this occurrence initializes no table.
fn initializer_brace(text: &str, from: usize) -> Option<usize> {
    for (offset, c) in text[from..].char_indices() {
        match c {
            '{' => return Some(from + offset),
            '=' | '[' | ']' | '*' | '_' => {}
            c if c.is_alphanumeric() || c.is_whitespace() => {}
            _ => return None,
        }
    }
    None
}

/// The name each entry of a table initializer registers, skipping the
/// terminating `{NULL, NULL, 0}` and anything unrecognized.
fn table_entry_names(body: &str) -> impl Iterator<Item = String> {
    split_top_level_commas(body)
        .into_iter()
        .filter_map(entry_name)
}

/// One table entry's R-visible name.
fn entry_name(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if let Some(rest) = entry.strip_prefix('{') {
        // `{"name", (DL_FUNC) &fn, n}` — the leading string literal is the name
        // R registers, and need not match the C function's.
        let first = split_top_level_commas(rest).into_iter().next()?;
        string_literal(first.trim())
    } else {
        // `CALLDEF(fn, n)` — a macro that stringifies its first argument into
        // the same shape. The spelling of the macro varies per package; what is
        // universal is that the routine comes first.
        let open = entry.find('(')?;
        let head = entry[..open].trim();
        if head.is_empty() || !head.chars().all(is_c_ident_char) {
            return None;
        }
        let close = entry.rfind(')')?;
        let first = split_top_level_commas(entry.get(open + 1..close)?)
            .into_iter()
            .next()?;
        let first = first.trim();
        string_literal(first).or_else(|| plausible_name(first))
    }
}

/// Unquote a C string literal, or `None` if `text` isn't one. Escapes are
/// resolved just enough for a routine name (`\\` and `\"`).
fn string_literal(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `text` as a routine name, if it could be one. Rules out the `NULL` of a
/// terminating entry and anything that isn't a bare identifier.
fn plausible_name(text: &str) -> Option<String> {
    if text.is_empty() || text == "NULL" {
        return None;
    }
    let mut chars = text.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '.' || c == '_');
    (first_ok && chars.all(is_c_ident_char)).then(|| text.to_string())
}

fn is_c_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// Whether the `len` bytes at `at` are bounded by non-identifier characters, so
/// `R_CMethodDef` doesn't match inside a longer name.
fn is_whole_word(text: &str, at: usize, len: usize) -> bool {
    let before = text[..at].chars().next_back();
    let after = text[at + len..].chars().next();
    !before.is_some_and(is_c_ident_char) && !after.is_some_and(is_c_ident_char)
}

/// The `}` closing the brace at `open`, string-aware.
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Split on commas outside every bracket and string literal.
fn split_top_level_commas(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(&text[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    parts.push(&text[start..]);
    parts
}

/// Replace every comment and preprocessor directive with a single space,
/// leaving string literals alone. This is what lets the scanners above treat the
/// source as flat text: a commented-out entry then contributes no name, and a
/// table fenced into `#ifdef`/`#endif` sections still reads as one comma-
/// separated list.
///
/// A conditionally compiled entry is therefore harvested whether or not this
/// build would compile it. That over-inclusion only ever *suppresses* an
/// `undefined-symbol`, and the R code guarding such a routine has to tolerate
/// its absence anyway.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    // Start of the run copied verbatim once the next elision is reached. Every
    // cut lands on an ASCII byte, so it is always a `char` boundary.
    let mut run = 0;
    let mut i = 0;
    // Whether only whitespace has been seen since the last newline, which is
    // what makes a `#` a directive rather than a stringify/paste operator.
    let mut at_line_start = true;
    while i < bytes.len() {
        match bytes[i] {
            // Scanned through, not elided. Doing it here is what keeps a `//`
            // or a `#` *inside* a literal from reading as syntax.
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 1 + bytes.get(i + 1).map_or(0, |&b| char_len(b));
                        continue;
                    }
                    let closed = bytes[i] == quote;
                    i += 1;
                    if closed {
                        break;
                    }
                }
                i = i.min(bytes.len());
                at_line_start = false;
            }
            // A directive runs to end of line, `\`-continued.
            b'#' if at_line_start => {
                out.push_str(&source[run..i]);
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1 + usize::from(bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\n'));
                }
                out.push(' ');
                run = i;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                out.push_str(&source[run..i]);
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push(' ');
                run = i;
                at_line_start = false;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                out.push_str(&source[run..i]);
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                out.push(' ');
                run = i;
                at_line_start = false;
            }
            b'\n' => {
                at_line_start = true;
                i += 1;
            }
            b => {
                at_line_start &= b.is_ascii_whitespace();
                i += 1;
            }
        }
    }
    out.push_str(&source[run..]);
    out
}

/// The byte length of the UTF-8 sequence starting with `first`.
fn char_len(first: u8) -> usize {
    match first {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> Vec<String> {
        registered_routines(source).into_iter().collect()
    }

    #[test]
    fn reads_a_string_literal_table() {
        // The shape `package_native_routine_registration_skeleton()` writes.
        let src = "\
static const R_CallMethodDef CallEntries[] = {
    {\"c_all_missing\", (DL_FUNC) &c_all_missing, 1},
    {\"c_qassert\",     (DL_FUNC) &c_qassert,     3},
    {NULL, NULL, 0}
};
";
        assert_eq!(names(src), ["c_all_missing", "c_qassert"]);
    }

    #[test]
    fn reads_a_stringifying_macro_table() {
        let src = "\
#define CALLDEF(name, n)  {#name, (DL_FUNC) &name, n}
static R_CallMethodDef CallEntries[] = {
    CALLDEF(R_all0, 1),
    CALLDEF(m_encodeInd,  4),
    {NULL, NULL, 0}
};
";
        assert_eq!(names(src), ["R_all0", "m_encodeInd"]);
    }

    #[test]
    fn reads_every_table_interface() {
        let src = "\
static const R_CMethodDef CEntries[] = {{\"c_one\", (DL_FUNC) &one, 1}, {NULL, NULL, 0}};
static const R_CallMethodDef CallEntries[] = {{\"call_one\", (DL_FUNC) &two, 1}, {NULL, NULL, 0}};
static const R_FortranMethodDef FEntries[] = {{\"f_one\", (DL_FUNC) &three, 1}, {NULL, NULL, 0}};
static const R_ExternalMethodDef Ext[] = {{\"ext_one\", (DL_FUNC) &four, 1}, {NULL, NULL, 0}};
";
        assert_eq!(names(src), ["c_one", "call_one", "ext_one", "f_one"]);
    }

    #[test]
    fn registered_name_may_differ_from_the_c_function() {
        // R registers the *string*, so `ffi_zap_srcref` is the binding even
        // though it forwards to `zap_srcref` (an rlang entry).
        let src =
            "static const R_CallMethodDef t[] = {{\"ffi_zap_srcref\", (DL_FUNC) &zap_srcref, 1}};";
        assert_eq!(names(src), ["ffi_zap_srcref"]);
    }

    #[test]
    fn skips_comments_and_forward_declarations() {
        // A commented-out entry registers nothing, and an `extern` declaration
        // of the table is not an initializer.
        let src = "\
extern const R_CallMethodDef CallEntries[];
static const R_CallMethodDef CallEntries[] = {
    {\"live\", (DL_FUNC) &live, 1},
    /* {\"dead\", (DL_FUNC) &dead, 1}, */
    // {\"gone\", (DL_FUNC) &gone, 1},
    {NULL, NULL, 0}
};
";
        assert_eq!(names(src), ["live"]);
    }

    #[test]
    fn reads_through_conditional_compilation() {
        // rlang fences one entry into an `#ifdef`. The directive lines are not
        // part of the initializer, and the entry is harvested either way — a
        // name that isn't compiled in only ever suppresses a finding.
        let src = "\
static const R_CallMethodDef t[] = {
    {\"ffi_set_names\", (DL_FUNC) &ffi_set_names, 4},
#ifdef RLANG_USE_PRIVATE_ACCESSORS
    {\"ffi_sexp_iterate\", (DL_FUNC) &ffi_sexp_iterate, 2},
#endif
    {\"ffi_squash\", (DL_FUNC) &ffi_squash, 4},
    {NULL, NULL, 0}
};
";
        assert_eq!(
            names(src),
            ["ffi_set_names", "ffi_sexp_iterate", "ffi_squash"]
        );
    }

    #[test]
    fn ignores_a_prototype_that_only_mentions_the_type() {
        // A parameter list is not a table, so nothing after it is harvested.
        let src = "void reg(DllInfo *dll, const R_CallMethodDef *tab) { tab_use(tab); }";
        assert!(names(src).is_empty());
    }

    #[test]
    fn ignores_a_longer_identifier_containing_a_type_name() {
        let src = "static const my_R_CallMethodDef_wrapper t[] = {{\"nope\", 0, 0}};";
        assert!(names(src).is_empty());
    }

    #[test]
    fn harvests_recursively_under_src() {
        // rlang's table lives in `src/internal/internal.c`.
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("src").join("internal");
        std::fs::create_dir_all(&nested).expect("src/internal");
        std::fs::write(
            nested.join("internal.c"),
            "static const R_CallMethodDef r_callables[] = {{\"ffi_enquo\", (DL_FUNC) &ffi_enquo, 2}, {NULL, NULL, 0}};",
        )
        .expect("internal.c");
        std::fs::write(dir.path().join("src").join("notes.txt"), "R_CallMethodDef")
            .expect("notes.txt");

        let found = dynlib_bound_names("useDynLib(rlang, .registration = TRUE)\n", dir.path());
        assert_eq!(found, ["ffi_enquo".to_string()].into());
    }

    #[test]
    fn fixes_wrap_every_harvested_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(
            dir.path().join("src").join("init.c"),
            "static const R_CallMethodDef t[] = {{\"rle\", (DL_FUNC) &rle, 1}, {NULL, NULL, 0}};",
        )
        .expect("init.c");

        let found = dynlib_bound_names(
            "useDynLib(bit, .registration = TRUE, .fixes = \"C_\")\n",
            dir.path(),
        );
        assert_eq!(found, ["C_rle".to_string()].into());
    }

    #[test]
    fn explicit_routines_need_no_c_sources() {
        // No `src/` at all: the NAMESPACE enumerates the bindings itself.
        let dir = tempfile::tempdir().expect("tempdir");
        let found = dynlib_bound_names("useDynLib(backports, dotsElt)\n", dir.path());
        assert_eq!(found, ["dotsElt".to_string()].into());
    }

    #[test]
    fn no_use_dyn_lib_binds_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(dynlib_bound_names("export(foo)\n", dir.path()).is_empty());
    }
}
