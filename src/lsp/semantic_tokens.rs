//! `textDocument/semanticTokens/full`: scope-aware highlighting that augments
//! the editor's grammar.
//!
//! We emit tokens *only* for identifiers that single-file scope analysis can add
//! value to — functions, variables, parameters, properties, and package
//! namespaces — and leave keywords, strings, numbers, comments, and operators to
//! the editor's own grammar (so highlighting degrades gracefully). Classification
//! reuses the [`SemanticModel`] (binding kinds + read resolution) plus the same
//! structural CST shapes the model builder keys on (call callees, `::`/`:::`
//! operands, `$`/`@` members, and named-argument names).
//!
//! This is a pure single-file computation with no salsa db, so the handler runs
//! straight on the read pool like `documentSymbol`/`foldingRange`.

use super::*;

/// The token types we emit, in legend order. An index into this array is what
/// goes on the wire as `SemanticToken::token_type`.
const TOKEN_TYPES: [SemanticTokenType; 5] = [
    SemanticTokenType::FUNCTION,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::NAMESPACE,
];

/// The token modifiers we emit, in legend order. A bit index into this array is
/// what `SemanticToken::token_modifiers_bitset` is built from.
const TOKEN_MODIFIERS: [SemanticTokenModifier; 1] = [SemanticTokenModifier::DEFINITION];

/// `definition` modifier bit — set on every binding *definition* site.
const MOD_DEFINITION: u32 = 1 << 0;

/// One emitted token kind, paired with its legend index via [`TokKind::index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Function,
    Variable,
    Parameter,
    Property,
    Namespace,
}

impl TokKind {
    /// Index into [`TOKEN_TYPES`] — kept in lockstep with that array's order.
    fn index(self) -> u32 {
        match self {
            TokKind::Function => 0,
            TokKind::Variable => 1,
            TokKind::Parameter => 2,
            TokKind::Property => 3,
            TokKind::Namespace => 4,
        }
    }
}

/// The legend handed to the client in `initialize`. The encoder and this legend
/// share [`TOKEN_TYPES`]/[`TOKEN_MODIFIERS`], so they can't drift.
pub(crate) fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Compute the full semantic-token set for `text`.
///
/// Parses, builds a [`SemanticModel`], then walks every `IDENT` token in source
/// order and classifies it. Definition sites win first (by range), then
/// structural shapes the model doesn't record as reads (callees, `::`/`$`
/// operands, arg names), then plain reads via scope resolution.
pub fn compute_semantic_tokens(text: &str, encoding: PositionEncoding) -> SemanticTokens {
    let parsed = parse(text);
    let root = &parsed.cst;
    let model = SemanticModel::build(root);
    let line_index = LineIndex::new(text);

    // Definition sites, keyed by the defining identifier's range.
    let mut def_kinds: HashMap<TextRange, TokKind> = HashMap::new();
    for binding in model.bindings() {
        let kind = match binding.kind {
            BindingKind::Param => TokKind::Parameter,
            BindingKind::ForVar => TokKind::Variable,
            BindingKind::Local | BindingKind::Implicit => {
                if def_is_function(root, binding.def_range) {
                    TokKind::Function
                } else {
                    TokKind::Variable
                }
            }
        };
        def_kinds.insert(binding.def_range, kind);
    }

    // Read sites, keyed by range: parameter reads stay parameters, everything
    // else (resolved local, or free) is a variable.
    let mut read_kinds: HashMap<TextRange, TokKind> = HashMap::new();
    for ident in model.idents() {
        let kind = match model.resolve_local(ident) {
            Some(id) if model.binding(id).kind == BindingKind::Param => TokKind::Parameter,
            _ => TokKind::Variable,
        };
        read_kinds.insert(ident.range, kind);
    }

    // Package names attached via `library()`/`require()` — suppressed as reads
    // by the model builder, but worth marking as namespaces.
    let mut package_ranges: HashMap<TextRange, TokKind> = HashMap::new();
    for pkg in model.loaded_packages() {
        package_ranges.insert(pkg.range, TokKind::Namespace);
    }

    let mut raw: Vec<(TextRange, TokKind, u32)> = Vec::new();
    for element in root.descendants_with_tokens() {
        let NodeOrToken::Token(tok) = element else {
            continue;
        };
        if tok.kind() != SyntaxKind::IDENT {
            continue;
        }
        let name = tok.text();
        // `...`, `..1`, and reserved literal constants (`TRUE`/`NA`/`NULL`/…)
        // lex as IDENT but aren't symbols — leave them to the grammar.
        if is_dot_dot(name) || crate::parser::expr::ident_is_special_constant(name) {
            continue;
        }
        let range = tok.text_range();
        if let Some(&kind) = def_kinds.get(&range) {
            raw.push((range, kind, MOD_DEFINITION));
        } else if let Some((kind, mods)) = classify_structural(&tok) {
            raw.push((range, kind, mods));
        } else if let Some(&kind) = read_kinds.get(&range) {
            raw.push((range, kind, 0));
        } else if let Some(&kind) = package_ranges.get(&range) {
            raw.push((range, kind, 0));
        } else {
            // A bare identifier we couldn't otherwise place: a free read.
            raw.push((range, TokKind::Variable, 0));
        }
    }

    encode(&line_index, &raw, encoding)
}

/// Whether `name` is a dot-dot identifier (`...`, `..1`) — lexed as IDENT but
/// not a scope-resolvable symbol. Mirrors the model builder's filter.
fn is_dot_dot(name: &str) -> bool {
    name.starts_with('.') && name.chars().all(|c| c == '.' || c.is_ascii_digit())
}

/// Classify an `IDENT` by its structural position — the shapes the model builder
/// deliberately does *not* record as reads. `None` when the token isn't one of
/// these shapes (the caller falls back to read resolution).
fn classify_structural(tok: &SyntaxToken<RLanguage>) -> Option<(TokKind, u32)> {
    let parent = tok.parent()?;
    let range = tok.text_range();
    match parent.kind() {
        SyntaxKind::CALL_EXPR => {
            let call = CallExpr::cast(parent)?;
            (call.callee_token().map(|t| t.text_range()) == Some(range))
                .then_some((TokKind::Function, 0))
        }
        SyntaxKind::BINARY_EXPR => {
            // Only the access operators mark a structural position; other binary
            // operators leave both operands as ordinary reads. Access operators
            // bind tightest, so `BinaryExpr::op` is the access token when present.
            let op = BinaryExpr::cast(parent)?.op()?;
            if !matches!(
                op.kind(),
                SyntaxKind::COLON2 | SyntaxKind::COLON3 | SyntaxKind::DOLLAR | SyntaxKind::AT
            ) {
                return None;
            }
            let before_op = range.start() < op.text_range().start();
            match op.kind() {
                // `pkg::name`: the package (LHS) is a namespace; the bare name
                // (RHS, no call) is a value we can't classify further.
                SyntaxKind::COLON2 | SyntaxKind::COLON3 => Some(if before_op {
                    (TokKind::Namespace, 0)
                } else {
                    (TokKind::Variable, 0)
                }),
                // `obj$field` / `obj@slot`: only the member name (RHS) is special.
                SyntaxKind::DOLLAR | SyntaxKind::AT => {
                    (!before_op).then_some((TokKind::Property, 0))
                }
                _ => None,
            }
        }
        SyntaxKind::ARG => is_arg_name(&parent, range).then_some((TokKind::Parameter, 0)),
        _ => None,
    }
}

/// Whether the identifier at `range` is the *name* of a named argument
/// (`name = value`) in `arg`. Mirrors the model builder's arg-name detection:
/// the single `IDENT`/`STRING` before `=`, with only trivia around it.
fn is_arg_name(arg: &SyntaxNode, range: TextRange) -> bool {
    let elements: Vec<_> = arg.children_with_tokens().collect();
    let Some(eq) = elements
        .iter()
        .position(|e| matches!(e, NodeOrToken::Token(t) if t.kind() == SyntaxKind::ASSIGN_EQ))
    else {
        return false;
    };
    let mut name_count = 0;
    let mut name_range = None;
    for el in &elements[..eq] {
        match el.kind() {
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT => {}
            SyntaxKind::IDENT | SyntaxKind::STRING => {
                name_count += 1;
                name_range = Some(el.text_range());
            }
            _ => return false,
        }
    }
    name_count == 1 && name_range == Some(range)
}

/// Whether the binding defined at `def_range` has a function (or lambda) value —
/// i.e. its enclosing assignment's right-hand side is a `FUNCTION_EXPR`. Inlines
/// the logic of `project::exports::classify_def` (which is private).
fn def_is_function(root: &SyntaxNode, def_range: TextRange) -> bool {
    let start = match root.covering_element(def_range) {
        NodeOrToken::Node(node) => node,
        NodeOrToken::Token(token) => match token.parent() {
            Some(parent) => parent,
            None => return false,
        },
    };
    for ancestor in start.ancestors() {
        if let Some(assign) = AssignmentExpr::cast(ancestor) {
            return matches!(
                assign.value_element(),
                Some(NodeOrToken::Node(value)) if FunctionExpr::can_cast(value.kind())
            );
        }
    }
    false
}

/// Delta-encode classified tokens (already in source order) into the LSP wire
/// format. Multi-line or zero-width tokens are dropped (identifiers are neither).
fn encode(
    line_index: &LineIndex,
    raw: &[(TextRange, TokKind, u32)],
    encoding: PositionEncoding,
) -> SemanticTokens {
    let mut data = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for (range, kind, mods) in raw {
        let start = line_index.byte_to_position(u32::from(range.start()) as usize, encoding);
        let end = line_index.byte_to_position(u32::from(range.end()) as usize, encoding);
        if end.line != start.line {
            continue;
        }
        let length = end.character.saturating_sub(start.character);
        if length == 0 {
            continue;
        }
        let delta_line = start.line - prev_line;
        let delta_start = if delta_line == 0 {
            start.character - prev_start
        } else {
            start.character
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: kind.index(),
            token_modifiers_bitset: *mods,
        });
        prev_line = start.line;
        prev_start = start.character;
    }
    SemanticTokens {
        result_id: None,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode the delta-encoded stream back into absolute
    /// `(line, char, length, type_index, modifiers)` tuples.
    fn decode(tokens: &SemanticTokens) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut out = Vec::new();
        let mut line = 0u32;
        let mut ch = 0u32;
        for t in &tokens.data {
            if t.delta_line == 0 {
                ch += t.delta_start;
            } else {
                line += t.delta_line;
                ch = t.delta_start;
            }
            out.push((line, ch, t.length, t.token_type, t.token_modifiers_bitset));
        }
        out
    }

    // Legend indices, for readable assertions.
    const FUNCTION: u32 = 0;
    const VARIABLE: u32 = 1;
    const PARAMETER: u32 = 2;
    const PROPERTY: u32 = 3;
    const NAMESPACE: u32 = 4;

    #[test]
    fn legend_order_matches_indices() {
        assert_eq!(TokKind::Function.index(), FUNCTION);
        assert_eq!(TokKind::Variable.index(), VARIABLE);
        assert_eq!(TokKind::Parameter.index(), PARAMETER);
        assert_eq!(TokKind::Property.index(), PROPERTY);
        assert_eq!(TokKind::Namespace.index(), NAMESPACE);
        assert_eq!(TOKEN_TYPES.len(), 5);
    }

    #[test]
    fn function_def_params_and_calls() {
        // `f` function+definition, `x` parameter+definition, `g` function call,
        // `x` (read) parameter.
        let toks = decode(&compute_semantic_tokens(
            "f <- function(x) g(x)",
            PositionEncoding::Utf16,
        ));
        assert_eq!(
            toks,
            vec![
                (0, 0, 1, FUNCTION, MOD_DEFINITION),
                (0, 14, 1, PARAMETER, MOD_DEFINITION),
                (0, 17, 1, FUNCTION, 0),
                (0, 19, 1, PARAMETER, 0),
            ]
        );
    }

    #[test]
    fn namespace_access_call() {
        // `pkg` namespace, `h` function (callee), `y` variable.
        let toks = decode(&compute_semantic_tokens(
            "pkg::h(y)",
            PositionEncoding::Utf16,
        ));
        assert_eq!(
            toks,
            vec![
                (0, 0, 3, NAMESPACE, 0),
                (0, 5, 1, FUNCTION, 0),
                (0, 7, 1, VARIABLE, 0),
            ]
        );
    }

    #[test]
    fn member_access() {
        // `obj` variable, `field` property.
        let toks = decode(&compute_semantic_tokens(
            "obj$field",
            PositionEncoding::Utf16,
        ));
        assert_eq!(toks, vec![(0, 0, 3, VARIABLE, 0), (0, 4, 5, PROPERTY, 0)]);
    }

    #[test]
    fn named_argument_name() {
        // `plot` function, `data` parameter (arg name), `d` variable.
        let toks = decode(&compute_semantic_tokens(
            "plot(data = d)",
            PositionEncoding::Utf16,
        ));
        assert_eq!(
            toks,
            vec![
                (0, 0, 4, FUNCTION, 0),
                (0, 5, 4, PARAMETER, 0),
                (0, 12, 1, VARIABLE, 0),
            ]
        );
    }

    #[test]
    fn for_loop_variable() {
        // `i` variable+definition, `xs` variable, `i` (read) variable.
        let toks = decode(&compute_semantic_tokens(
            "for (i in xs) i",
            PositionEncoding::Utf16,
        ));
        assert_eq!(
            toks,
            vec![
                (0, 5, 1, VARIABLE, MOD_DEFINITION),
                (0, 10, 2, VARIABLE, 0),
                (0, 14, 1, VARIABLE, 0),
            ]
        );
    }

    #[test]
    fn reserved_constants_emit_no_token() {
        // Only `f` is emitted; `TRUE`/`NULL` are reserved literals.
        let toks = decode(&compute_semantic_tokens(
            "f(TRUE, NULL)",
            PositionEncoding::Utf16,
        ));
        assert_eq!(toks, vec![(0, 0, 1, FUNCTION, 0)]);
    }

    #[test]
    fn library_package_is_namespace() {
        // `library` function, `dplyr` namespace (not a read).
        let toks = decode(&compute_semantic_tokens(
            "library(dplyr)",
            PositionEncoding::Utf16,
        ));
        assert_eq!(toks, vec![(0, 0, 7, FUNCTION, 0), (0, 8, 5, NAMESPACE, 0)]);
    }
}
