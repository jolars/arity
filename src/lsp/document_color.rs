//! `textDocument/documentColor` + `textDocument/colorPresentation`: inline color
//! swatches for string literals and hex presentations for the editor's picker.
//!
//! A pure single-file CST walk (like `documentSymbol`/`foldingRange`): every
//! single-line string literal whose entire content is a `#RRGGBB`/`#RRGGBBAA` hex
//! code or a `grDevices::colors()` name (matched case-insensitively, as base R's
//! `col2rgb` resolves `"Red"`) becomes a [`ColorInformation`]. `colorPresentation`
//! answers the picker with a single hex presentation whose edit rewrites the
//! literal in place, preserving its quote character.

use super::*;

use super::color_names::lookup_named;
use crate::project::collect_string_literals;

/// An 8-bit RGBA color: the common currency between R color spellings (hex or
/// named, always 8-bit) and the LSP [`Color`] (four `f32` channels in `[0, 1]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rgba {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Rgba {
    /// The LSP color, each channel scaled to `[0, 1]`.
    fn to_lsp(self) -> Color {
        let f = |c: u8| c as f32 / 255.0;
        Color {
            red: f(self.r),
            green: f(self.g),
            blue: f(self.b),
            alpha: f(self.a),
        }
    }

    /// The nearest 8-bit color to an LSP [`Color`] (channels clamped, rounded).
    fn from_lsp(color: &Color) -> Rgba {
        let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba {
            r: q(color.red),
            g: q(color.green),
            b: q(color.blue),
            a: q(color.alpha),
        }
    }

    /// The hex spelling: `#rrggbb` when fully opaque, else `#rrggbbaa`. Lowercase,
    /// matching common R style and `grDevices` output.
    fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex color (case-insensitive). Anchored: the
/// whole string must be `#` followed by exactly 6 or 8 hex digits. The 3/4-digit
/// short form is rejected, matching base R (`col2rgb("#fff")` errors).
fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    // `h` is ASCII here, so byte offsets and char offsets coincide.
    match h.len() {
        6 => Some(Rgba {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: 255,
        }),
        8 => Some(Rgba {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: byte(6)?,
        }),
        _ => None,
    }
}

/// The color a string literal's content denotes, if any: a hex code first, else a
/// named `grDevices` color (fully opaque). `None` for everything else.
fn color_of_content(content: &str) -> Option<Rgba> {
    if let Some(rgba) = parse_hex(content) {
        return Some(rgba);
    }
    let (r, g, b) = lookup_named(content)?;
    Some(Rgba { r, g, b, a: 255 })
}

/// The color swatches in `text`: every single-line string literal whose content
/// is a recognized hex or named color. A pure CST walk — no semantic model, no
/// workspace snapshot — so it runs straight on the read pool.
pub fn compute_document_colors(text: &str) -> Vec<ColorInformation> {
    let root = parse(text).cst;
    let line_index = LineIndex::new(text);
    collect_string_literals(&root)
        .into_iter()
        .filter_map(|literal| {
            let rgba = color_of_content(&literal.spelling)?;
            let range = text_range_to_lsp_range(&line_index, literal.literal_range);
            // Editors draw the swatch on a single line; skip any literal that
            // straddles a newline (e.g. a multi-line raw string).
            (range.start.line == range.end.line).then(|| ColorInformation {
                range,
                color: rgba.to_lsp(),
            })
        })
        .collect()
}

/// The picker presentations for `color` at `range`. `range` is the one we emitted
/// in [`compute_document_colors`]: the whole string token, quotes included. So the
/// edit replaces that span with a *quoted* hex spelling (replacing content-only
/// would orphan the quotes). The original quote is recovered by peeking `text` at
/// the range start, defaulting to `"` — which also normalizes raw strings and
/// covers the case where the buffer changed since the range was produced.
///
/// A single hex presentation, mirroring the R `languageserver`. `label` is the
/// bare hex (a clean picker header); the edit inserts the quoted form.
pub fn compute_color_presentations(
    text: &str,
    color: &Color,
    range: Range,
) -> Vec<ColorPresentation> {
    let line_index = LineIndex::new(text);
    let start = line_index.position_to_byte(range.start);
    let quote = if text.as_bytes().get(start) == Some(&b'\'') {
        '\''
    } else {
        '"'
    };
    let hex = Rgba::from_lsp(color).to_hex();
    let new_text = format!("{quote}{hex}{quote}");
    vec![ColorPresentation {
        label: hex,
        text_edit: Some(TextEdit { range, new_text }),
        additional_text_edits: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_digit_hex_opaque() {
        assert_eq!(
            parse_hex("#ff0000"),
            Some(Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
    }

    #[test]
    fn parses_eight_digit_hex_with_alpha() {
        assert_eq!(
            parse_hex("#ff000080"),
            Some(Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            })
        );
    }

    #[test]
    fn hex_parse_is_case_insensitive() {
        assert_eq!(parse_hex("#FF0000"), parse_hex("#ff0000"));
    }

    #[test]
    fn rejects_short_and_malformed_hex() {
        assert_eq!(parse_hex("#fff"), None); // 3-digit short form
        assert_eq!(parse_hex("#ff00"), None); // 4-digit short form
        assert_eq!(parse_hex("#gggggg"), None); // non-hex
        assert_eq!(parse_hex("ff0000"), None); // no leading '#'
        assert_eq!(parse_hex("#ff0000 "), None); // trailing junk
    }

    #[test]
    fn content_recognizes_hex_and_named_colors() {
        assert_eq!(
            color_of_content("#ff0000").map(|c| (c.r, c.g, c.b, c.a)),
            Some((255, 0, 0, 255))
        );
        assert_eq!(
            color_of_content("red").map(|c| (c.r, c.g, c.b)),
            Some((255, 0, 0))
        );
        assert_eq!(
            color_of_content("Red").map(|c| (c.r, c.g, c.b)),
            Some((255, 0, 0))
        );
        assert_eq!(color_of_content("hello"), None);
    }

    #[test]
    fn rgba_round_trips_through_lsp() {
        let c = Rgba {
            r: 1,
            g: 128,
            b: 255,
            a: 64,
        };
        assert_eq!(Rgba::from_lsp(&c.to_lsp()), c);
    }

    #[test]
    fn hex_output_drops_alpha_when_opaque() {
        assert_eq!(
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
            .to_hex(),
            "#ff0000"
        );
        assert_eq!(
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            }
            .to_hex(),
            "#ff000080"
        );
    }
}
