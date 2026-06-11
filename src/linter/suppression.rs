//! Comment-based suppression: `# arity-ignore` directives.
//!
//! Three forms are recognized:
//!
//! ```text
//! # arity-ignore <rule>: <reason>           # suppresses on the next non-trivia sibling
//! # arity-ignore-file <rule>: <reason>      # suppresses anywhere in the file
//! # arity-ignore-file: <reason>             # suppresses ALL rules
//! ```
//!
//! Implementation note: the comment-to-node attachment for a node-level
//! suppression is "next non-trivia sibling", computed during the walk. This
//! avoids the rowan/biome `place_comment` indirection jarl had to write.

use std::collections::{HashMap, HashSet};

use rowan::{NodeOrToken, TextRange};

use crate::syntax::{SyntaxKind, SyntaxNode};

#[derive(Debug, Clone, Default)]
pub struct SuppressionMap {
    /// Rule IDs suppressed file-wide (`# arity-ignore-file <rule>: …`).
    file_rules: HashSet<String>,
    /// Whether the file has a "suppress everything" directive.
    file_all: bool,
    /// (rule, range) pairs. A diagnostic is suppressed if its range falls
    /// fully inside any of the registered ranges for its rule.
    node_skips: HashMap<String, Vec<TextRange>>,
}

impl SuppressionMap {
    pub fn build(root: &SyntaxNode) -> Self {
        let mut map = Self::default();
        visit(root, &mut map);
        map
    }

    pub fn is_suppressed(&self, rule: &str, range: TextRange) -> bool {
        if self.file_all {
            return true;
        }
        if self.file_rules.contains(rule) {
            return true;
        }
        if let Some(ranges) = self.node_skips.get(rule) {
            for r in ranges {
                if r.contains_range(range) {
                    return true;
                }
            }
        }
        false
    }
}

fn visit(node: &SyntaxNode, map: &mut SuppressionMap) {
    // Walk every comment token in the file. Comments may appear as direct
    // children of any node or as part of trivia between siblings. We use a
    // descendants iterator on tokens.
    for el in node.descendants_with_tokens() {
        if let NodeOrToken::Token(tok) = el
            && tok.kind() == SyntaxKind::COMMENT
        {
            classify_comment(&tok, map);
        }
    }
}

fn classify_comment(tok: &rowan::SyntaxToken<crate::syntax::RLanguage>, map: &mut SuppressionMap) {
    let text = tok.text();
    let body = match text.strip_prefix('#') {
        Some(rest) => rest.trim_start(),
        None => return,
    };
    if let Some(rest) = body.strip_prefix("arity-ignore-file") {
        let rest = rest.trim_start();
        if let Some(rule_part) = rest.strip_prefix(':') {
            let _ = rule_part;
            map.file_all = true;
            return;
        }
        // `arity-ignore-file <rule>: reason` — rest starts with the rule ID.
        if let Some(rule) = parse_rule(rest) {
            map.file_rules.insert(rule);
        }
        return;
    }
    if let Some(rest) = body.strip_prefix("arity-ignore") {
        let rest = rest.trim_start();
        if let Some(rule) = parse_rule(rest) {
            // Attach to the next non-trivia, non-comment sibling.
            if let Some(target) = next_meaningful_sibling(tok) {
                map.node_skips.entry(rule).or_default().push(target);
            }
        }
    }
}

fn parse_rule(rest: &str) -> Option<String> {
    // Expect `<rule>: ...` or just `<rule>` (lone trailing whitespace).
    let trimmed = rest.trim_start();
    let end = trimmed
        .find(|c: char| c == ':' || c.is_whitespace())
        .unwrap_or(trimmed.len());
    if end == 0 {
        return None;
    }
    let rule = trimmed[..end].to_string();
    if rule.is_empty() {
        return None;
    }
    Some(rule)
}

fn next_meaningful_sibling(
    tok: &rowan::SyntaxToken<crate::syntax::RLanguage>,
) -> Option<TextRange> {
    // The "next sibling" is the next non-trivia, non-comment element after
    // this token within its parent node. We expand outward if the parent is
    // itself trivia-only — e.g. a comment between top-level statements lives
    // under ROOT, and the next sibling is the next top-level expression.
    let mut current_token = tok.clone();
    loop {
        let parent = current_token.parent()?;
        let mut found = None;
        let mut past_self = false;
        for el in parent.children_with_tokens() {
            match &el {
                NodeOrToken::Token(t) if *t == current_token => {
                    past_self = true;
                    continue;
                }
                _ => {}
            }
            if !past_self {
                continue;
            }
            match &el {
                NodeOrToken::Token(t)
                    if matches!(
                        t.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    ) =>
                {
                    continue;
                }
                NodeOrToken::Node(child) => {
                    found = Some(child.text_range());
                    break;
                }
                NodeOrToken::Token(t) => {
                    found = Some(t.text_range());
                    break;
                }
            }
        }
        if let Some(range) = found {
            return Some(range);
        }
        // No sibling after this token in `parent`. Bubble up: look for the
        // next non-trivia sibling of `parent` itself.
        let parent_node = parent.clone();
        let grand = parent_node.parent()?;
        let mut past_parent = false;
        for el in grand.children_with_tokens() {
            match &el {
                NodeOrToken::Node(n) if *n == parent_node => {
                    past_parent = true;
                    continue;
                }
                _ => {}
            }
            if !past_parent {
                continue;
            }
            match &el {
                NodeOrToken::Token(t)
                    if matches!(
                        t.kind(),
                        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
                    ) =>
                {
                    continue;
                }
                NodeOrToken::Node(child) => return Some(child.text_range()),
                NodeOrToken::Token(t) => return Some(t.text_range()),
            }
        }
        // Try one level higher.
        current_token = grand.first_token()?;
        // Prevent infinite loops.
        if grand == parent {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn map_of(src: &str) -> SuppressionMap {
        let parsed = parse(src);
        SuppressionMap::build(&parsed.cst)
    }

    #[test]
    fn file_all_suppresses_everything() {
        let m = map_of("# arity-ignore-file: noisy\nx <- 1\n");
        assert!(m.is_suppressed("anything", TextRange::new(0.into(), 1.into())));
    }

    #[test]
    fn file_rule_suppresses_only_that_rule() {
        let m = map_of("# arity-ignore-file unused-binding: temp\nx <- 1\n");
        assert!(m.is_suppressed("unused-binding", TextRange::new(0.into(), 1.into())));
        assert!(!m.is_suppressed("undefined-symbol", TextRange::new(0.into(), 1.into())));
    }

    #[test]
    fn node_suppression_attaches_to_next_sibling() {
        let src = "# arity-ignore unused-binding: temp\nx <- 1\n";
        let m = map_of(src);
        // The `x <- 1` ASSIGNMENT_EXPR spans 36..42 in the file.
        assert!(m.is_suppressed("unused-binding", TextRange::new(36.into(), 42.into())));
    }

    #[test]
    fn node_suppression_does_not_leak_to_following_statements() {
        let src = "# arity-ignore unused-binding: only first\nx <- 1\ny <- 2\n";
        let m = map_of(src);
        // x assignment is at 42..48, y at 49..55.
        assert!(m.is_suppressed("unused-binding", TextRange::new(42.into(), 48.into())));
        assert!(!m.is_suppressed("unused-binding", TextRange::new(49.into(), 55.into())));
    }
}
