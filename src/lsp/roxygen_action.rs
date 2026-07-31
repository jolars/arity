//! CST-driven code action that generates or extends a roxygen2 documentation
//! block for the function under the cursor.
//!
//! Unlike the quick-fixes in [`super::code_actions`], this action is *not* backed
//! by a lint diagnostic: it is a cursor-context refactor computed straight from
//! the parsed CST. It offers a single, context-aware action:
//!
//! * **Add** — when no roxygen block precedes the function, insert a skeleton
//!   (`#' Title`, one `#' @param` per formal, `#' @return`) directly above it.
//! * **Update** — when a block already precedes the function but some formals are
//!   undocumented, insert only the missing `#' @param` lines, in formal order.
//!
//! The action is non-destructive: existing prose and tags are never rewritten or
//! removed, so the edit stays parse-clean and lossless (the autofix-correctness
//! bar). Layout is the formatter's job (Tenet 1); this only emits a reasonable
//! skeleton.

use super::*;
use crate::ast::{Param, RoxygenBlock};

/// Compute the roxygen "Add/Update documentation" code action for the function
/// touched by `range`, if any. Pure over the in-memory `text` so it can be
/// unit-tested like [`super::compute_code_actions`].
pub(crate) fn roxygen_code_action(
    text: &str,
    uri: &Uri,
    range: Range,
    encoding: PositionEncoding,
) -> Option<CodeActionOrCommand> {
    let line_index = LineIndex::new(text);
    let offset = line_index.position_to_byte(range.start, encoding);
    let root = parse(text).cst;
    let token = token_at(&root, TextSize::new(offset as u32))?;

    // Locate the function: the cursor may be inside the `function(...)` body, or
    // on the binding (name/operator) whose value is the function.
    let node = token.parent()?;
    let func = node.ancestors().find_map(FunctionExpr::cast).or_else(|| {
        node.ancestors()
            .find_map(AssignmentExpr::cast)
            .and_then(|a| a.value_element())
            .and_then(|v| v.into_node())
            .and_then(FunctionExpr::cast)
    })?;

    // Require a simple binding (`name <- function(...)`); the function must be the
    // direct value of the assignment, not nested inside a call on the RHS.
    let assign = func.syntax().ancestors().find_map(AssignmentExpr::cast)?;
    if assign.value_element().and_then(|v| v.into_node()).as_ref() != Some(func.syntax()) {
        return None;
    }
    assign.target_name()?;

    let params = func.params();
    let stmt = assign.syntax();
    let stmt_start = usize::from(stmt.text_range().start());
    let indent = leading_indent(text, stmt_start);

    let (edit_start, new_text, verb) = match preceding_roxygen_block(stmt) {
        None => (stmt_start, build_skeleton(&params, &indent), "Add"),
        Some(block) => {
            let (offset, content) = build_update(text, &block, &params, &indent)?;
            (offset, content, "Update")
        }
    };

    let pos = line_index.byte_to_position(edit_start, encoding);
    let edit = TextEdit {
        range: Range {
            start: pos,
            end: pos,
        },
        new_text,
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![edit]);
    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("{verb} roxygen documentation"),
        kind: Some(CodeActionKind::REFACTOR),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    }))
}

/// The token at (or just before) `offset`, mirroring the helper in
/// `src/project/classes.rs`.
fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken<RLanguage>> {
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(t) => Some(t),
        // At a boundary (e.g. a cursor at column 0), prefer the content token on
        // the right over a preceding newline/whitespace.
        TokenAtOffset::Between(l, r) => Some(
            if matches!(l.kind(), SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE) {
                r
            } else {
                l
            },
        ),
    }
}

/// The leading whitespace (indentation) of the source line containing `start`.
fn leading_indent(text: &str, start: usize) -> String {
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..start]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The roxygen block immediately preceding `stmt`, if one is attached to it. A
/// blank line between the block and the statement detaches it (roxygen2's rule),
/// so that yields `None`.
fn preceding_roxygen_block(stmt: &SyntaxNode) -> Option<RoxygenBlock> {
    let mut newlines = 0usize;
    let mut prev = stmt.prev_sibling_or_token();
    while let Some(el) = prev {
        match el {
            NodeOrToken::Token(t) => match t.kind() {
                SyntaxKind::NEWLINE => {
                    newlines += t.text().matches('\n').count();
                    if newlines > 1 {
                        return None;
                    }
                    prev = t.prev_sibling_or_token();
                }
                SyntaxKind::WHITESPACE => prev = t.prev_sibling_or_token(),
                _ => return None,
            },
            NodeOrToken::Node(n) => return RoxygenBlock::cast(n),
        }
    }
    None
}

/// Build a fresh roxygen skeleton (title + one `@param` per formal + `@return`),
/// each line prefixed with `indent` and terminated by a newline so the block
/// sits directly above the function's statement.
fn build_skeleton(params: &[Param], indent: &str) -> String {
    let mut lines = vec!["#' Title".to_string(), "#'".to_string()];
    if !params.is_empty() {
        for p in params {
            lines.push(format!("#' @param {}", p.name));
        }
        lines.push("#'".to_string());
    }
    lines.push("#' @return".to_string());

    let mut out = String::new();
    for line in &lines {
        out.push_str(indent);
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Build the update edit: the byte offset to insert at and the `@param` lines for
/// formals not already documented. `None` when every formal is documented.
fn build_update(
    text: &str,
    block: &RoxygenBlock,
    params: &[Param],
    indent: &str,
) -> Option<(usize, String)> {
    let mut documented = HashSet::new();
    for tag in block.tags() {
        if tag.name().as_deref() == Some("param") {
            for (name, _) in tag.arg_names() {
                documented.insert(name.to_string());
            }
        }
    }

    let missing: Vec<&Param> = params
        .iter()
        .filter(|p| !documented.contains(p.name.as_str()))
        .collect();
    if missing.is_empty() {
        return None;
    }

    let anchor = trim_trailing_ws(text, insertion_offset(block));
    let mut content = String::new();
    for p in &missing {
        content.push('\n');
        content.push_str(indent);
        content.push_str(&format!("#' @param {}", p.name));
    }
    Some((anchor, content))
}

/// The byte offset to anchor inserted `@param` lines after: the end of the last
/// existing `@param` section, else the end of the intro (untagged) section, else
/// the end of the block.
fn insertion_offset(block: &RoxygenBlock) -> usize {
    let mut last_param_end = None;
    let mut intro_end = None;
    for section in block.sections() {
        let end = usize::from(section.syntax().text_range().end());
        match section.tag() {
            Some(tag) if tag.name().as_deref() == Some("param") => last_param_end = Some(end),
            None => intro_end = Some(end),
            _ => {}
        }
    }
    last_param_end
        .or(intro_end)
        .unwrap_or_else(|| usize::from(block.syntax().text_range().end()))
}

/// Move `end` back over trailing whitespace and newlines so it points just after
/// the last visible content character. Section ranges include a trailing newline
/// only when the section is not the block's last, so this normalizes both.
fn trim_trailing_ws(text: &str, mut end: usize) -> usize {
    let bytes = text.as_bytes();
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invoke the action with the cursor on the first line (where the `function`
    /// keyword lives in these fixtures).
    fn action_on_line_0(src: &str) -> Option<CodeAction> {
        let range = Range {
            start: pos(0, 0),
            end: pos(0, 0),
        };
        // Place the cursor on the line holding the `function` keyword.
        let line = src[..src.find("function").unwrap()].matches('\n').count() as u32;
        let range = Range {
            start: pos(line, 0),
            ..range
        };
        match roxygen_code_action(src, &test_uri(), range, PositionEncoding::Utf16)? {
            CodeActionOrCommand::CodeAction(a) => Some(a),
            _ => None,
        }
    }

    /// The sole edit's replacement text and the document after applying it.
    fn applied(src: &str, action: &CodeAction) -> (String, String) {
        let (range, new_text) = sole_edit(action.edit.as_ref().unwrap(), &test_uri());
        let li = LineIndex::new(src);
        let start = li.position_to_byte(range.start, PositionEncoding::Utf16);
        let end = li.position_to_byte(range.end, PositionEncoding::Utf16);
        let mut out = src.to_string();
        out.replace_range(start..end, &new_text);
        (new_text, out)
    }

    fn assert_lossless(src: &str) {
        assert_eq!(crate::parser::reconstruct(src), src, "must round-trip");
    }

    #[test]
    fn add_inserts_skeleton_for_undocumented_function() {
        let src = "f <- function(x, y) x\n";
        let action = action_on_line_0(src).expect("an add action");
        assert_eq!(action.title, "Add roxygen documentation");
        assert_eq!(action.kind, Some(CodeActionKind::REFACTOR));

        let (new_text, out) = applied(src, &action);
        assert!(new_text.contains("#' Title"));
        assert!(new_text.contains("#' @param x\n"));
        assert!(new_text.contains("#' @param y\n"));
        assert!(new_text.contains("#' @return\n"));
        assert_eq!(
            out,
            "#' Title\n#'\n#' @param x\n#' @param y\n#'\n#' @return\nf <- function(x, y) x\n"
        );
        assert_lossless(&out);
    }

    #[test]
    fn add_skeleton_has_no_param_lines_for_nullary_function() {
        let src = "f <- function() 1\n";
        let action = action_on_line_0(src).expect("an add action");
        let (_, out) = applied(src, &action);
        assert_eq!(out, "#' Title\n#'\n#' @return\nf <- function() 1\n");
        assert_lossless(&out);
    }

    #[test]
    fn update_inserts_only_missing_param() {
        let src = "#' Title\n#' @param x foo\nf <- function(x, y) x\n";
        let action = action_on_line_0(src).expect("an update action");
        assert_eq!(action.title, "Update roxygen documentation");

        let (new_text, out) = applied(src, &action);
        assert!(!new_text.contains("@param x"), "must not re-add x");
        assert!(new_text.contains("#' @param y"));
        assert_eq!(
            out,
            "#' Title\n#' @param x foo\n#' @param y\nf <- function(x, y) x\n"
        );
        assert_lossless(&out);
    }

    #[test]
    fn update_anchors_params_after_intro_when_none_documented() {
        let src = "#' Title\n#' @return bar\nf <- function(x) x\n";
        let action = action_on_line_0(src).expect("an update action");
        let (_, out) = applied(src, &action);
        assert_eq!(
            out,
            "#' Title\n#' @param x\n#' @return bar\nf <- function(x) x\n"
        );
        assert_lossless(&out);
    }

    #[test]
    fn no_action_when_all_params_documented() {
        let src = "#' Title\n#' @param x foo\nf <- function(x) x\n";
        assert!(action_on_line_0(src).is_none());
    }

    #[test]
    fn no_action_for_anonymous_function() {
        let src = "lapply(xs, function(v) v + 1)\n";
        assert!(action_on_line_0(src).is_none());
    }

    #[test]
    fn no_action_when_function_nested_in_call_on_rhs() {
        let src = "y <- purrr::compose(function(v) v)\n";
        assert!(action_on_line_0(src).is_none());
    }
}
