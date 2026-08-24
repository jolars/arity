//! Row-wise table layout for `tribble()` calls.
//!
//! `tibble::tribble()` declares its columns with leading one-sided formulas and
//! then lists the cells row-major, so the call *is* a table and reads as one
//! only when it is laid out as one. This module turns such a call into a header
//! line plus one line per row, with cells padded to a common column width and
//! numeric columns aligned on their decimal point.
//!
//! Two things distinguish this from air's `fmt: table`, which is otherwise the
//! reference for the alignment arithmetic here:
//!
//! * Rows come from the **header count**, never from where the author happened
//!   to put line breaks (Tenet: input line breaks never influence output). A
//!   `tribble()` written on one line lays out as a table all the same.
//! * The table is not a general facility keyed on a directive or a configurable
//!   name list. It applies to `tribble` alone, bare or `::`-qualified.
//!
//! Layout is all-or-nothing. Anything that would make the row shape a guess — a
//! ragged cell count, a hole, a named argument, dynamic dots, a cell that cannot
//! render on one line — declines the table and lets ordinary call formatting
//! run, so no input is ever laid out as a table that is not one.

use rowan::{NodeOrToken, SyntaxElement, SyntaxToken};

use super::super::context::FormatContext;
use super::super::core::{ir_expr_segment, is_trivia};
use super::super::ir::Ir;
use super::super::printer::Printer;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

/// The width of the decimal point that separates a numeric cell's two halves.
/// An integer literal's `L` suffix occupies the same position, which is why
/// `1L` lines up under `1.5`.
const DOT_WIDTH: usize = 1;

/// Build the argument list of a `tribble()` call as an aligned table, or return
/// `None` to let the caller format the call the ordinary way.
///
/// `callee` is the run of elements before the `(`; `arg_list` the `ARG_LIST`
/// node. The returned IR covers `(` through `)`, exactly like the arg-list IR
/// the ordinary path builds.
///
/// Total by construction: the table is an opportunistic layout laid over a call
/// the ordinary path can already format, so every way of not producing one —
/// including a cell the expression builder rejects — is a `None`, never an
/// error that would fail the whole file.
pub(crate) fn ir_tribble_table(
    callee: &[SyntaxElement<RLanguage>],
    arg_list: &SyntaxNode,
    indent: usize,
    ctx: FormatContext,
) -> Option<Ir> {
    if !callee_is_tribble(callee) {
        return None;
    }
    let (args, trailing_comma) = collect_args(arg_list)?;
    if args.iter().any(is_dynamic_dots) {
        return None;
    }

    // Leading `~name` formulas declare the columns; everything after them is
    // cell data, which must divide evenly into rows of that width.
    let columns = args
        .iter()
        .take_while(|arg| is_one_sided_formula(arg))
        .count();
    if columns == 0 || (args.len() - columns) % columns != 0 {
        return None;
    }

    let printer = Printer::new(ctx.style());
    let mut cells: Vec<Cell> = Vec::with_capacity(args.len());
    for arg in &args {
        cells.push(build_cell(arg, &printer, indent, ctx)?);
    }

    let widths = measure_columns(&cells, columns);
    Some(render(&cells, &widths, columns, trailing_comma))
}

/// Whether the call's callee names `tribble`: a bare `tribble`, or a
/// `pkg::tribble` / `pkg:::tribble` qualification. `$` and `@` extraction do not
/// count — they select a value at run time and say nothing statically about
/// which function is called.
fn callee_is_tribble(elements: &[SyntaxElement<RLanguage>]) -> bool {
    match significant(elements.iter().cloned()).as_slice() {
        [NodeOrToken::Token(tok)] => tok.kind() == SyntaxKind::IDENT && tok.text() == "tribble",
        [NodeOrToken::Node(node)] if node.kind() == SyntaxKind::BINARY_EXPR => {
            let parts = significant(node.children_with_tokens());
            matches!(
                parts.as_slice(),
                [_, NodeOrToken::Token(op), NodeOrToken::Token(name)]
                    if matches!(op.kind(), SyntaxKind::COLON2 | SyntaxKind::COLON3)
                        && name.kind() == SyntaxKind::IDENT
                        && name.text() == "tribble"
            )
        }
        _ => false,
    }
}

/// The call's arguments in order, plus whether the list ends on a trailing
/// comma. Returns `None` when the list holds anything the table cannot place: a
/// hole (`tribble(~x, , 1)`), a named argument, or an argument that is not a
/// single expression.
fn collect_args(arg_list: &SyntaxNode) -> Option<(Vec<SyntaxNode>, bool)> {
    let mut slots: Vec<Option<SyntaxNode>> = Vec::new();
    let mut current: Option<SyntaxNode> = None;
    for element in arg_list.children_with_tokens() {
        match element {
            NodeOrToken::Node(arg) if arg.kind() == SyntaxKind::ARG => {
                if arg.children_with_tokens().all(|el| is_trivia(el.kind())) {
                    continue;
                }
                current = Some(arg);
            }
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::COMMA => {
                slots.push(current.take());
            }
            _ => {}
        }
    }
    slots.push(current.take());

    // A trailing comma leaves a final empty slot. It is punctuation the author
    // chose, not a cell, so it is kept as such and every *interior* hole still
    // declines the table.
    let trailing_comma = slots.len() > 1 && slots.last().is_some_and(Option::is_none);
    if trailing_comma {
        slots.pop();
    }

    let mut args = Vec::with_capacity(slots.len());
    for slot in slots {
        let arg = slot?;
        // A named argument (`tribble(~x, 1, .rows = 2)`) is not a cell.
        if arg
            .children_with_tokens()
            .any(|el| el.kind() == SyntaxKind::ASSIGN_EQ)
        {
            return None;
        }
        args.push(arg);
    }
    if args.is_empty() {
        return None;
    }
    Some((args, trailing_comma))
}

/// Whether an argument is a one-sided formula `~name`, i.e. a column header.
fn is_one_sided_formula(arg: &SyntaxNode) -> bool {
    let Some(NodeOrToken::Node(node)) = single_significant(arg) else {
        return false;
    };
    unary_parts(&node).is_some_and(|(op, _)| op.kind() == SyntaxKind::TILDE)
}

/// Whether an argument splices an unknown number of cells into the call, so the
/// row shape is not statically known: rlang's `!!`/`!!!` (which parse as
/// repeated `!`) or a bare `...` forwarding.
fn is_dynamic_dots(arg: &SyntaxNode) -> bool {
    match single_significant(arg) {
        Some(NodeOrToken::Token(tok)) => tok.kind() == SyntaxKind::IDENT && tok.text() == "...",
        Some(NodeOrToken::Node(node)) => unary_parts(&node).is_some_and(|(op, operand)| {
            op.kind() == SyntaxKind::BANG
                && matches!(&operand, NodeOrToken::Node(inner)
                    if unary_parts(inner).is_some_and(|(op, _)| op.kind() == SyntaxKind::BANG))
        }),
        None => false,
    }
}

/// One rendered table cell: the single line it occupies, and how that line is
/// aligned within its column.
struct Cell {
    text: String,
    kind: CellKind,
}

enum CellKind {
    /// A numeric literal, optionally signed. Right-aligned, and aligned on the
    /// decimal point when its column holds one.
    Numeric {
        integer_width: usize,
        /// `None` for a literal with no decimal point at all (`250`), which
        /// therefore has nothing to align *on*; `Some(0)` for `1.` and for the
        /// `L` of `1L`.
        fractional_width: Option<usize>,
    },
    /// Anything else. Left-aligned.
    Other,
}

impl Cell {
    fn width(&self) -> usize {
        // Count characters, not bytes: alignment is about the columns a reader
        // sees, and cells are routinely non-ASCII (`"A (Mio. €)"`).
        self.text.chars().count()
    }
}

/// Format one argument and, if it lays out on a single line, classify it for
/// alignment. `None` means it cannot sit in a row: it carries a forced break (a
/// block, a comment, an embedded newline), or it is not a shape the expression
/// builder handles at all.
fn build_cell(
    arg: &SyntaxNode,
    printer: &Printer,
    indent: usize,
    ctx: FormatContext,
) -> Option<Cell> {
    let element = single_significant(arg)?;
    let elements: Vec<_> = arg.children_with_tokens().collect();
    let ir = ir_expr_segment(&elements, "tribble cell", indent, ctx).ok()?;
    if ir.contains_forced_break() {
        return None;
    }
    let text = printer.render_flat(&ir)?;

    // Classify from the *rendered* text rather than the source token, so the
    // measured halves always describe the bytes that actually land in the row.
    let kind = if is_numeric_atom(&element) {
        numeric_kind(&text)
    } else {
        CellKind::Other
    };
    Some(Cell { text, kind })
}

/// Whether an element is a numeric literal, optionally behind a single `+`/`-`.
/// A repeated unary (`--1`) is deliberately excluded: the sign run is no longer
/// a fixed one-column prefix, so the cell aligns as ordinary text instead.
fn is_numeric_atom(element: &SyntaxElement<RLanguage>) -> bool {
    match element {
        NodeOrToken::Token(tok) => is_numeric_literal(tok),
        NodeOrToken::Node(node) => unary_parts(node).is_some_and(|(op, operand)| {
            matches!(op.kind(), SyntaxKind::PLUS | SyntaxKind::MINUS)
                && matches!(&operand, NodeOrToken::Token(tok) if is_numeric_literal(tok))
        }),
    }
}

fn is_numeric_literal(token: &SyntaxToken<RLanguage>) -> bool {
    matches!(token.kind(), SyntaxKind::INT | SyntaxKind::FLOAT)
}

/// Split a rendered numeric literal into the halves that align on the decimal
/// point.
fn numeric_kind(text: &str) -> CellKind {
    let width = text.chars().count();
    let integer_width = text.chars().take_while(|c| *c != '.').count();
    let (integer_width, fractional_width) = if integer_width < width {
        (integer_width, Some(width - integer_width - DOT_WIDTH))
    } else if text.ends_with('L') {
        // An integer literal's `L` sits where the decimal point would, so it
        // aligns with one.
        (width - 1, Some(0))
    } else {
        (width, None)
    };
    CellKind::Numeric {
        integer_width,
        fractional_width,
    }
}

/// How wide each column's parts are, across all rows.
#[derive(Default)]
struct ColumnInfo {
    /// Whether any cell in the column has a decimal point to align on.
    has_decimal: bool,
    /// The widest rendered cell, whatever its kind.
    max_width: usize,
    max_integer_part: usize,
    max_fractional_part: usize,
}

fn measure_columns(cells: &[Cell], columns: usize) -> Vec<ColumnInfo> {
    let mut infos: Vec<ColumnInfo> = (0..columns).map(|_| ColumnInfo::default()).collect();
    for (index, cell) in cells.iter().enumerate() {
        let info = &mut infos[index % columns];
        info.max_width = info.max_width.max(cell.width());
        if let CellKind::Numeric {
            integer_width,
            fractional_width,
        } = cell.kind
        {
            info.max_integer_part = info.max_integer_part.max(integer_width);
            if let Some(fractional_width) = fractional_width {
                info.has_decimal = true;
                info.max_fractional_part = info.max_fractional_part.max(fractional_width);
            }
        }
    }
    infos
}

impl ColumnInfo {
    /// The spaces to place before and after a cell so it fills its column.
    fn padding(&self, cell: &Cell) -> (usize, usize) {
        if self.has_decimal {
            self.decimal_padding(cell)
        } else {
            self.simple_padding(cell)
        }
    }

    /// Without a decimal point to align on, every cell is simply padded to the
    /// column width: numbers to the right, everything else to the left.
    fn simple_padding(&self, cell: &Cell) -> (usize, usize) {
        let padding = self.max_width - cell.width();
        match cell.kind {
            CellKind::Numeric { .. } => (padding, 0),
            CellKind::Other => (0, padding),
        }
    }

    /// With a decimal point in the column, the numeric cells form a sub-column
    /// whose points line up. A non-numeric cell may be wider than that
    /// sub-column, so everything is padded out to whichever is wider; otherwise
    /// the commas after the column would not align.
    fn decimal_padding(&self, cell: &Cell) -> (usize, usize) {
        let decimal_width = self.max_integer_part + DOT_WIDTH + self.max_fractional_part;
        let target = self.max_width.max(decimal_width);
        match cell.kind {
            CellKind::Numeric {
                integer_width,
                fractional_width,
            } => {
                let left = self.max_integer_part - integer_width;
                let right = match fractional_width {
                    Some(width) => self.max_fractional_part - width,
                    // No point of its own: the whole `.` + fraction field is
                    // blank for this cell.
                    None => DOT_WIDTH + self.max_fractional_part,
                };
                (left, right + (target - decimal_width))
            }
            CellKind::Other => (0, target - cell.width()),
        }
    }
}

/// Lay the cells out as `(`, one indented line per row, then `)`.
///
/// Padding goes *before* each comma so the commas line up as their own column,
/// matching air. The very last cell is padded only when a trailing comma
/// follows it, so no line ever ends in whitespace.
fn render(cells: &[Cell], widths: &[ColumnInfo], columns: usize, trailing_comma: bool) -> Ir {
    let mut lines: Vec<Ir> = Vec::new();
    for (row_index, row) in cells.chunks(columns).enumerate() {
        let mut parts: Vec<Ir> = Vec::new();
        for (column_index, cell) in row.iter().enumerate() {
            let column = &widths[column_index];
            let (left, right) = column.padding(cell);
            let is_final = row_index * columns + column_index + 1 == cells.len();
            parts.push(spaces(left));
            parts.push(Ir::text(cell.text.clone()));
            if !is_final || trailing_comma {
                parts.push(spaces(right));
                parts.push(Ir::text(" ,"));
                // The separator's own space belongs between cells, never at the
                // end of a line.
                if column_index + 1 < row.len() {
                    parts.push(Ir::text(" "));
                }
            }
        }
        lines.push(Ir::concat(parts));
    }

    let body = Ir::concat(
        lines
            .into_iter()
            .map(|line| Ir::concat([Ir::hard_line(), line])),
    );
    Ir::concat([
        Ir::text("("),
        Ir::indent(body),
        Ir::hard_line(),
        Ir::text(")"),
    ])
}

fn spaces(count: usize) -> Ir {
    if count == 0 {
        Ir::nil()
    } else {
        Ir::text(" ".repeat(count))
    }
}

/// The elements of `iter` that carry meaning, i.e. everything but trivia.
fn significant(
    iter: impl Iterator<Item = SyntaxElement<RLanguage>>,
) -> Vec<SyntaxElement<RLanguage>> {
    iter.filter(|el| !is_trivia(el.kind())).collect()
}

/// The one significant child of `node`, or `None` when it has several (or none).
fn single_significant(node: &SyntaxNode) -> Option<SyntaxElement<RLanguage>> {
    match significant(node.children_with_tokens()).as_slice() {
        [element] => Some(element.clone()),
        _ => None,
    }
}

/// The operator token and operand of a unary expression.
fn unary_parts(node: &SyntaxNode) -> Option<(SyntaxToken<RLanguage>, SyntaxElement<RLanguage>)> {
    if node.kind() != SyntaxKind::UNARY_EXPR {
        return None;
    }
    match significant(node.children_with_tokens()).as_slice() {
        [NodeOrToken::Token(op), operand] => Some((op.clone(), operand.clone())),
        _ => None,
    }
}
