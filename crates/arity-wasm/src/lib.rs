//! Browser bindings for Arity's R formatter and parse diagnostics.

use arity_formatter::FormatStyle;
use arity_parser::parser::{ParseDiagnostic, parse};
use wasm_bindgen::prelude::*;

/// A parse diagnostic using JavaScript's UTF-16 string positions.
#[derive(Debug, Clone, PartialEq, Eq)]
#[wasm_bindgen(getter_with_clone)]
pub struct Diagnostic {
    pub message: String,
    /// Zero-based UTF-16 start offset.
    pub start: u32,
    /// Exclusive UTF-16 end offset.
    pub end: u32,
    /// One-based line number.
    pub line: u32,
    /// One-based UTF-16 column.
    pub column: u32,
}

/// Return parse errors without evaluating the R source.
#[wasm_bindgen]
pub fn check(input: &str) -> Vec<Diagnostic> {
    to_diagnostics(input, parse(input).diagnostics)
}

/// Format R source using Arity's defaults for omitted options.
#[wasm_bindgen]
pub fn format(
    input: &str,
    line_width: Option<u32>,
    indent_width: Option<u32>,
    roxygen: Option<bool>,
) -> Result<String, JsError> {
    format_impl(input, line_width, indent_width, roxygen).map_err(|error| JsError::new(&error))
}

fn format_impl(
    input: &str,
    line_width: Option<u32>,
    indent_width: Option<u32>,
    roxygen: Option<bool>,
) -> Result<String, String> {
    let mut style = FormatStyle::default();
    for (name, value, target) in [
        ("line-width", line_width, &mut style.line_width),
        ("indent-width", indent_width, &mut style.indent_width),
    ] {
        if let Some(value) = value {
            if !(1..=1000).contains(&value) {
                return Err(format!("{name} must be between 1 and 1000, got {value}"));
            }
            *target = value as usize;
        }
    }
    if let Some(roxygen) = roxygen {
        style.roxygen = roxygen;
    }
    arity_formatter::format_with_style(input, style).map_err(|error| error.to_string())
}

fn to_diagnostics(input: &str, errors: Vec<ParseDiagnostic>) -> Vec<Diagnostic> {
    let mut wanted = Vec::with_capacity(errors.len() * 2);
    let mut diagnostics = Vec::with_capacity(errors.len());
    for (index, error) in errors.into_iter().enumerate() {
        wanted.push((error.start, index, false));
        wanted.push((error.end, index, true));
        diagnostics.push(Diagnostic {
            message: error.message,
            start: 0,
            end: 0,
            line: 1,
            column: 1,
        });
    }
    if wanted.is_empty() {
        return diagnostics;
    }
    // Recovery can report spans out of order. Translate all endpoints in one
    // source walk without changing the diagnostic order.
    wanted.sort_unstable_by_key(|&(byte, ..)| byte);
    let mut record = |index: usize, is_end: bool, offset: u32, line: u32, column: u32| {
        let diagnostic = &mut diagnostics[index];
        if is_end {
            diagnostic.end = offset;
        } else {
            diagnostic.start = offset;
            diagnostic.line = line;
            diagnostic.column = column;
        }
    };
    let (mut offset, mut line, mut column) = (0, 1, 1);
    let mut next = 0;
    for (byte, ch) in input.char_indices() {
        while let Some(&(target, index, is_end)) = wanted.get(next) {
            if target > byte {
                break;
            }
            record(index, is_end, offset, line, column);
            next += 1;
        }
        offset += ch.len_utf16() as u32;
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += ch.len_utf16() as u32;
        }
    }
    for &(_, index, is_end) in &wanted[next..] {
        record(index, is_end, offset, line, column);
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use arity_formatter::{FormatStyle, format_with_style};

    #[test]
    fn defaults_match_the_formatter_and_remain_idempotent() {
        for source in ["", "x=1\n", "f=function(x){if(x>0){x+1}else{0}}\n"] {
            let formatted = format_impl(source, None, None, None).unwrap();
            assert_eq!(formatted, arity_formatter::format(source).unwrap());
            assert_eq!(
                format_impl(&formatted, None, None, None).unwrap(),
                formatted
            );
            assert!(check(&formatted).is_empty());
            assert_eq!(arity_parser::parser::reconstruct(source), source);
        }
    }

    #[test]
    fn custom_options_match_the_formatter() {
        let source = "f=function(x,y){list(first=x,second=y,third=x+y)}\n";
        let style = FormatStyle {
            line_width: 30,
            indent_width: 4,
            ..FormatStyle::default()
        };
        let formatted = format_impl(source, Some(30), Some(4), None).unwrap();
        assert_eq!(formatted, format_with_style(source, style).unwrap());
        assert!(formatted.contains("\n    "));
    }

    #[test]
    fn invalid_widths_are_errors() {
        for width in [0, 1001, u32::MAX] {
            assert!(
                format_impl("x=1", Some(width), None, None)
                    .unwrap_err()
                    .contains("line-width")
            );
            assert!(
                format_impl("x=1", None, Some(width), None)
                    .unwrap_err()
                    .contains("indent-width")
            );
        }
        for width in [1, 1000] {
            assert!(format_impl("x=1", Some(width), Some(width), None).is_ok());
        }
    }

    #[test]
    fn invalid_r_has_diagnostics_and_cannot_be_formatted() {
        let source = "f(\n";
        assert!(!check(source).is_empty());
        assert!(format_impl(source, None, None, None).is_err());
        assert_eq!(arity_parser::parser::reconstruct(source), source);
    }

    #[test]
    fn roxygen_can_be_preserved() {
        let source = "#' A title\n#' @param x   Some input.\nf=function(x){x}\n";
        let formatted = format_impl(source, None, None, Some(false)).unwrap();
        assert!(formatted.starts_with("#' A title\n#' @param x   Some input.\n"));
        let style = FormatStyle {
            roxygen: false,
            ..FormatStyle::default()
        };
        assert_eq!(formatted, format_with_style(source, style).unwrap());
    }

    #[test]
    fn directives_preserve_their_source() {
        let source = "# arity-format skip-file\r\nx=  1\r\n";
        assert_eq!(format_impl(source, None, None, None).unwrap(), source);
    }

    #[test]
    fn diagnostics_use_utf16_positions_even_when_out_of_order() {
        let input = "# 😀é\r\n)";
        let diagnostics = to_diagnostics(
            input,
            vec![
                ParseDiagnostic {
                    message: "closing".into(),
                    start: 10,
                    end: 11,
                },
                ParseDiagnostic {
                    message: "emoji".into(),
                    start: 2,
                    end: 6,
                },
            ],
        );
        assert_eq!(
            diagnostics,
            vec![
                Diagnostic {
                    message: "closing".into(),
                    start: 7,
                    end: 8,
                    line: 2,
                    column: 1
                },
                Diagnostic {
                    message: "emoji".into(),
                    start: 2,
                    end: 4,
                    line: 1,
                    column: 3
                },
            ]
        );
        assert_eq!(
            check(input),
            vec![Diagnostic {
                message: "unexpected ')'".into(),
                start: 7,
                end: 8,
                line: 2,
                column: 1
            },]
        );
    }

    #[test]
    fn empty_and_eof_diagnostic_spans_are_supported() {
        for input in ["", "# 😀\n"] {
            let diagnostics = to_diagnostics(
                input,
                vec![ParseDiagnostic {
                    message: "at end".into(),
                    start: input.len(),
                    end: input.len(),
                }],
            );
            let offset = input.encode_utf16().count() as u32;
            assert_eq!(diagnostics.len(), 1);
            assert_eq!((diagnostics[0].start, diagnostics[0].end), (offset, offset));
            assert_eq!(diagnostics[0].line, if input.is_empty() { 1 } else { 2 });
            assert_eq!(diagnostics[0].column, 1);
        }
    }
}
