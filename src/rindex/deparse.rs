//! Deparse a default-value [`Robj`] back to R source text.
//!
//! A function's formal default is stored **unevaluated** in the lazy-load DB —
//! a literal is a vector SEXP, a bare name is a symbol, and anything compound
//! (`c(1, 2)`, `a + b`, `getOption("x")`) is a language object. This module
//! renders such an object back to tidyverse-spaced source text for storage in
//! the index (`Formal::default`), where it later feeds LSP signature help.
//!
//! It is total over every non-`Missing` input and never panics: shapes we do
//! not model fall back to `NULL`, and a depth cap guards pathological nesting.
//! Numeric literals are reformatted from their decoded value (the original
//! spelling — `1e5`, `0x10` — is not preserved), which stays valid R.

use std::fmt::Write as _;

use crate::rindex::rds::{Rkind, Robj};

/// Guard against pathological / cyclic nesting; far deeper than any real default.
const MAX_DEPTH: usize = 64;

/// Render a formal's default expression as R source text. Returns `None` for a
/// required argument (the empty-arg sentinel, `Rkind::Missing`).
pub fn deparse(obj: &Robj) -> Option<String> {
    if matches!(obj.kind, Rkind::Missing) {
        return None;
    }
    let mut out = String::new();
    write_obj(&mut out, obj, 0);
    Some(out)
}

fn write_obj(out: &mut String, obj: &Robj, depth: usize) {
    if depth > MAX_DEPTH {
        out.push_str("NULL");
        return;
    }
    match &obj.kind {
        Rkind::Null => out.push_str("NULL"),
        // A missing argument inside a call (e.g. `x[, 1]`) renders as nothing.
        Rkind::Missing => {}
        Rkind::Logical(v) => write_vec(out, v, "logical(0)", |out, x| {
            out.push_str(match x {
                Some(true) => "TRUE",
                Some(false) => "FALSE",
                None => "NA",
            });
        }),
        Rkind::Int(v) => write_vec(out, v, "integer(0)", |out, x| match x {
            Some(i) => {
                let _ = write!(out, "{i}L");
            }
            None => out.push_str("NA_integer_"),
        }),
        Rkind::Real(v) => write_vec(out, v, "numeric(0)", |out, x| write_real(out, *x)),
        Rkind::Str(v) => write_vec(out, v, "character(0)", |out, x| match x {
            Some(s) => write_string_lit(out, s),
            None => out.push_str("NA_character_"),
        }),
        Rkind::Symbol(s) => write_name(out, s),
        Rkind::List(v) => write_list(out, obj, v, depth),
        Rkind::Pairlist(items) => write_call(out, items, depth),
        // Defaults are never a compiled closure / builtin / environment / S4;
        // emit valid R rather than fail.
        Rkind::Closure { .. } | Rkind::Builtin | Rkind::Env | Rkind::Opaque => out.push_str("NULL"),
    }
}

/// Render a vector: a scalar prints its single literal; length > 1 wraps the
/// elements in `c(...)`; an empty vector uses the typed-constructor fallback.
fn write_vec<T>(out: &mut String, v: &[T], empty: &str, one: impl Fn(&mut String, &T)) {
    match v {
        [] => out.push_str(empty),
        [x] => one(out, x),
        _ => {
            out.push_str("c(");
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                one(out, x);
            }
            out.push(')');
        }
    }
}

fn write_real(out: &mut String, x: f64) {
    if x.is_nan() {
        out.push_str("NaN");
    } else if x.is_infinite() {
        out.push_str(if x < 0.0 { "-Inf" } else { "Inf" });
    } else if x == x.trunc() && x.abs() < 1e15 {
        // Integer-valued: print without a decimal point (and never `-0`).
        let i = x as i64;
        let _ = write!(out, "{i}");
    } else {
        // Rust's shortest round-trip formatting matches R for simple defaults.
        let _ = write!(out, "{x}");
    }
}

fn write_string_lit(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Write an identifier, backtick-quoting it when it is not a syntactic R name.
fn write_name(out: &mut String, name: &str) {
    if is_syntactic(name) {
        out.push_str(name);
    } else {
        out.push('`');
        out.push_str(name);
        out.push('`');
    }
}

fn is_syntactic(name: &str) -> bool {
    // The empty symbol (R's empty-arg placeholder) renders bare (as nothing).
    if name.is_empty() {
        return true;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    // R's "letter" is locale-dependent; in a UTF-8 locale any alphabetic
    // character starts a name, so `café` deparses bare (issue #108). Only ASCII
    // digits count as digits, as in `make.names`.
    if !(first.is_alphabetic() || first == '.') {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '_')
    {
        return false;
    }
    !is_reserved(name)
}

fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "repeat"
            | "while"
            | "function"
            | "for"
            | "in"
            | "next"
            | "break"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "Inf"
            | "NaN"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_character_"
            | "NA_complex_"
    )
}

fn write_list(out: &mut String, obj: &Robj, v: &[Robj], depth: usize) {
    out.push_str("list(");
    let names = obj.names();
    for (i, el) in v.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if let Some(Some(nm)) = names.as_ref().and_then(|ns| ns.get(i))
            && !nm.is_empty()
        {
            write_name(out, nm);
            out.push_str(" = ");
        }
        write_obj(out, el, depth + 1);
    }
    out.push(')');
}

/// Render a language object (a `LANGSXP` call): `items[0]` is the head, the
/// rest are arguments carrying optional tags (named-argument names).
fn write_call(out: &mut String, items: &[crate::rindex::rds::PairlistItem], depth: usize) {
    let Some((head, args)) = items.split_first() else {
        out.push_str("NULL");
        return;
    };
    if let Rkind::Symbol(op) = &head.value.kind
        && write_operator(out, op, args, depth)
    {
        return;
    }
    // Ordinary call: head(arg1, name = arg2, ...).
    write_head(out, &head.value, depth);
    out.push('(');
    write_args(out, args, depth);
    out.push(')');
}

type Item = crate::rindex::rds::PairlistItem;

/// Try to render `op(args)` using operator / special-form syntax. Returns
/// `false` (caller falls back to a generic call) when `op` is not an operator
/// in the relevant arity.
fn write_operator(out: &mut String, op: &str, args: &[Item], depth: usize) -> bool {
    match op {
        "$" | "@" if args.len() == 2 => {
            write_operand(out, &args[0].value, depth);
            out.push_str(op);
            write_obj(out, &args[1].value, depth + 1);
            true
        }
        "::" | ":::" if args.len() == 2 => {
            write_obj(out, &args[0].value, depth + 1);
            out.push_str(op);
            write_obj(out, &args[1].value, depth + 1);
            true
        }
        "[" if !args.is_empty() => {
            write_operand(out, &args[0].value, depth);
            out.push('[');
            write_args(out, &args[1..], depth);
            out.push(']');
            true
        }
        "[[" if !args.is_empty() => {
            write_operand(out, &args[0].value, depth);
            out.push_str("[[");
            write_args(out, &args[1..], depth);
            out.push_str("]]");
            true
        }
        "(" if args.len() == 1 => {
            out.push('(');
            write_obj(out, &args[0].value, depth + 1);
            out.push(')');
            true
        }
        "{" => {
            out.push_str("{ ");
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str("; ");
                }
                write_obj(out, &a.value, depth + 1);
            }
            out.push_str(" }");
            true
        }
        "if" if args.len() >= 2 => {
            out.push_str("if (");
            write_obj(out, &args[0].value, depth + 1);
            out.push_str(") ");
            write_obj(out, &args[1].value, depth + 1);
            if let Some(else_branch) = args.get(2) {
                out.push_str(" else ");
                write_obj(out, &else_branch.value, depth + 1);
            }
            true
        }
        "for" if args.len() == 3 => {
            out.push_str("for (");
            write_obj(out, &args[0].value, depth + 1);
            out.push_str(" in ");
            write_obj(out, &args[1].value, depth + 1);
            out.push_str(") ");
            write_obj(out, &args[2].value, depth + 1);
            true
        }
        "while" if args.len() == 2 => {
            out.push_str("while (");
            write_obj(out, &args[0].value, depth + 1);
            out.push_str(") ");
            write_obj(out, &args[1].value, depth + 1);
            true
        }
        "function" if args.len() >= 2 => {
            out.push_str("function(");
            write_formals(out, &args[0].value, depth);
            out.push_str(") ");
            write_obj(out, &args[1].value, depth + 1);
            true
        }
        "!" if args.len() == 1 => {
            out.push('!');
            write_operand(out, &args[0].value, depth);
            true
        }
        "-" | "+" if args.len() == 1 => {
            out.push_str(op);
            write_operand(out, &args[0].value, depth);
            true
        }
        "^" | ":" if args.len() == 2 => {
            // Tidyverse style: no spaces around `^` or `:`.
            write_operand(out, &args[0].value, depth);
            out.push_str(op);
            write_operand(out, &args[1].value, depth);
            true
        }
        _ if args.len() == 2 && is_binary_spaced(op) => {
            write_operand(out, &args[0].value, depth);
            out.push(' ');
            out.push_str(op);
            out.push(' ');
            write_operand(out, &args[1].value, depth);
            true
        }
        _ => false,
    }
}

fn is_binary_spaced(op: &str) -> bool {
    matches!(
        op,
        "+" | "-"
            | "*"
            | "/"
            | "%%"
            | "%/%"
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "&"
            | "&&"
            | "|"
            | "||"
            | "<-"
            | "<<-"
            | "="
            | "->"
            | "->>"
            | "~"
    ) || (op.starts_with('%') && op.ends_with('%') && op.len() >= 2)
}

/// Render an operand, parenthesizing it when it is itself an infix/unary
/// operator call that could otherwise rebind. Tight-binding forms (`$`, `@`,
/// indexing, `::`, an explicit `(...)`, and ordinary calls) never need parens.
fn write_operand(out: &mut String, obj: &Robj, depth: usize) {
    if needs_parens(obj) {
        out.push('(');
        write_obj(out, obj, depth + 1);
        out.push(')');
    } else {
        write_obj(out, obj, depth + 1);
    }
}

fn needs_parens(obj: &Robj) -> bool {
    let Rkind::Pairlist(items) = &obj.kind else {
        return false;
    };
    let Some(head) = items.first() else {
        return false;
    };
    let Rkind::Symbol(op) = &head.value.kind else {
        return false;
    };
    let args = items.len() - 1;
    match op.as_str() {
        "!" | "-" | "+" if args == 1 => true,
        "^" | ":" if args == 2 => true,
        _ => args == 2 && is_binary_spaced(op),
    }
}

fn write_head(out: &mut String, head: &Robj, depth: usize) {
    match &head.kind {
        Rkind::Symbol(s) => write_name(out, s),
        _ => write_obj(out, head, depth + 1),
    }
}

fn write_args(out: &mut String, args: &[Item], depth: usize) {
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if let Some(tag) = &a.tag {
            write_name(out, tag);
            out.push_str(" = ");
        }
        write_obj(out, &a.value, depth + 1);
    }
}

fn write_formals(out: &mut String, obj: &Robj, depth: usize) {
    let Rkind::Pairlist(items) = &obj.kind else {
        return;
    };
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_name(out, it.tag.as_deref().unwrap_or(""));
        if !matches!(it.value.kind, Rkind::Missing) {
            out.push_str(" = ");
            write_obj(out, &it.value, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::rds::PairlistItem;
    use smol_str::SmolStr;

    /// R's letters are locale-dependent, so a UTF-8 name is syntactic and
    /// deparses bare --- `deparse(quote(café))` is `café`, not `` `café` ``
    /// (issue #108).
    #[test]
    fn non_ascii_letters_deparse_without_backticks() {
        let mut out = String::new();
        write_name(&mut out, "café");
        write_name(&mut out, "日本語");
        assert_eq!(out, "café日本語");

        let mut out = String::new();
        write_name(&mut out, "😀");
        assert_eq!(out, "`😀`");
    }

    fn bare(kind: Rkind) -> Robj {
        Robj {
            kind,
            attr: Vec::new(),
        }
    }

    fn sym(name: &str) -> Robj {
        bare(Rkind::Symbol(SmolStr::new(name)))
    }

    /// A LANGSXP call `head(args...)` with optionally-tagged args.
    fn call(parts: Vec<(Option<&str>, Robj)>) -> Robj {
        bare(Rkind::Pairlist(
            parts
                .into_iter()
                .map(|(tag, value)| PairlistItem {
                    tag: tag.map(SmolStr::new),
                    value,
                })
                .collect(),
        ))
    }

    fn d(obj: &Robj) -> Option<String> {
        deparse(obj)
    }

    #[test]
    fn scalars_and_na() {
        assert_eq!(d(&bare(Rkind::Missing)), None);
        assert_eq!(d(&bare(Rkind::Null)).as_deref(), Some("NULL"));
        assert_eq!(
            d(&bare(Rkind::Logical(vec![Some(true)]))).as_deref(),
            Some("TRUE")
        );
        assert_eq!(d(&bare(Rkind::Logical(vec![None]))).as_deref(), Some("NA"));
        assert_eq!(d(&bare(Rkind::Int(vec![Some(1)]))).as_deref(), Some("1L"));
        assert_eq!(
            d(&bare(Rkind::Int(vec![None]))).as_deref(),
            Some("NA_integer_")
        );
        assert_eq!(d(&bare(Rkind::Real(vec![1.0]))).as_deref(), Some("1"));
        assert_eq!(d(&bare(Rkind::Real(vec![2.5]))).as_deref(), Some("2.5"));
        assert_eq!(
            d(&bare(Rkind::Str(vec![None]))).as_deref(),
            Some("NA_character_")
        );
    }

    #[test]
    fn string_escaping() {
        let s = bare(Rkind::Str(vec![Some("a\"b\\c".to_string())]));
        assert_eq!(d(&s).as_deref(), Some(r#""a\"b\\c""#));
    }

    #[test]
    fn vectors_use_c() {
        let v = bare(Rkind::Real(vec![1.0, 2.0]));
        assert_eq!(d(&v).as_deref(), Some("c(1, 2)"));
        let iv = bare(Rkind::Int(vec![Some(1), Some(2)]));
        assert_eq!(d(&iv).as_deref(), Some("c(1L, 2L)"));
    }

    #[test]
    fn symbols_and_backticks() {
        assert_eq!(d(&sym("foo")).as_deref(), Some("foo"));
        assert_eq!(d(&sym("a b")).as_deref(), Some("`a b`"));
    }

    #[test]
    fn generic_call_with_named_arg() {
        // c(1, 2)
        let c = call(vec![
            (None, sym("c")),
            (None, bare(Rkind::Real(vec![1.0]))),
            (None, bare(Rkind::Real(vec![2.0]))),
        ]);
        assert_eq!(d(&c).as_deref(), Some("c(1, 2)"));

        // f(x, y = 1)
        let f = call(vec![
            (None, sym("f")),
            (None, sym("x")),
            (Some("y"), bare(Rkind::Int(vec![Some(1)]))),
        ]);
        assert_eq!(d(&f).as_deref(), Some("f(x, y = 1L)"));

        // getOption("foo")
        let g = call(vec![
            (None, sym("getOption")),
            (None, bare(Rkind::Str(vec![Some("foo".to_string())]))),
        ]);
        assert_eq!(d(&g).as_deref(), Some(r#"getOption("foo")"#));
    }

    #[test]
    fn operators() {
        // unary minus: -1
        let neg = call(vec![(None, sym("-")), (None, bare(Rkind::Real(vec![1.0])))]);
        assert_eq!(d(&neg).as_deref(), Some("-1"));

        // x + 1
        let add = call(vec![
            (None, sym("+")),
            (None, sym("x")),
            (None, bare(Rkind::Real(vec![1.0]))),
        ]);
        assert_eq!(d(&add).as_deref(), Some("x + 1"));

        // a$b
        let dollar = call(vec![(None, sym("$")), (None, sym("a")), (None, sym("b"))]);
        assert_eq!(d(&dollar).as_deref(), Some("a$b"));

        // 1:10  (no spaces)
        let seq = call(vec![
            (None, sym(":")),
            (None, bare(Rkind::Int(vec![Some(1)]))),
            (None, bare(Rkind::Int(vec![Some(10)]))),
        ]);
        assert_eq!(d(&seq).as_deref(), Some("1L:10L"));

        // %in%: x %in% y
        let inop = call(vec![
            (None, sym("%in%")),
            (None, sym("x")),
            (None, sym("y")),
        ]);
        assert_eq!(d(&inop).as_deref(), Some("x %in% y"));
    }

    #[test]
    fn parenthesizes_nested_operators() {
        // (a + b) * c
        let inner = call(vec![(None, sym("+")), (None, sym("a")), (None, sym("b"))]);
        let outer = call(vec![(None, sym("*")), (None, inner), (None, sym("c"))]);
        assert_eq!(d(&outer).as_deref(), Some("(a + b) * c"));
    }

    #[test]
    fn pkg_qualified_call() {
        // stats::rnorm(n)
        let head = call(vec![
            (None, sym("::")),
            (None, sym("stats")),
            (None, sym("rnorm")),
        ]);
        let c = call(vec![(None, head), (None, sym("n"))]);
        assert_eq!(d(&c).as_deref(), Some("stats::rnorm(n)"));
    }
}
