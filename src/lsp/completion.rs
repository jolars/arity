//! Code completion: scope-aware bare names (locals + attached-package exports +
//! base R) and `pkg::`/`pkg:::` member completion, backed by the harvested
//! index. Items carry only a label + identity; docs/signature are attached
//! lazily on `completionItem/resolve`.
//!
//! Mirrors the hover read path: [`completion_via_db`] resolves off the snapshot's
//! cached parse when the tracked buffer matches, else re-parses; [`compute_completions`]
//! is the pure, parse-from-text wrapper used by tests. Completion stays strictly
//! read-only — an unharvested package degrades to the bundled names-only list,
//! and the normal lint cycle harvests `referenced_packages` so a later request
//! sees rich data.

use super::*;
use crate::syntax::SyntaxElement;

/// Identity carried on a completion item (serialized into `CompletionItem.data`)
/// so `completionItem/resolve` can attach docs without the original document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
enum CompletionData {
    /// A namespaced or attached-package symbol; resolve via `indexed.lookup`.
    Member { package: SmolStr, name: SmolStr },
    /// A bare base-R name; resolve via the base name→package map then lookup.
    Bare { name: SmolStr },
    /// A scope local; nothing to attach.
    Local,
    /// A `$`/`@` field harvested statically from usage or construction; nothing
    /// to attach (arity does not evaluate, so a field carries no signature/docs).
    Field,
}

/// What the cursor is positioned to complete.
enum CompletionContext {
    /// After `pkg::` / `pkg:::`, optionally with a partial member name.
    Member {
        package: SmolStr,
        internal: bool,
        prefix: String,
    },
    /// After `receiver$` / `receiver@`, optionally with a partial field name.
    /// `receiver` is the whitespace-normalized source text of the left operand;
    /// `at` is `true` for `@` (S4 slot) versus `$` (list/`$` access).
    Field {
        receiver: String,
        at: bool,
        prefix: String,
    },
    /// A bare identifier with a (possibly empty) typed prefix.
    Bare { prefix: String, offset: TextSize },
    /// Inside a string/comment, or nothing to complete.
    None,
}

/// A gathered candidate before it becomes a `CompletionItem`. `sort_group`
/// orders the buckets (0 locals, 1 attached lib, 2 base, 3 member).
struct Candidate {
    label: String,
    kind: CompletionItemKind,
    sort_group: u8,
    data: CompletionData,
    /// Origin shown as the dimmed label description (`dplyr`, `base`, `local`).
    /// Cheap to set at gather time; the signature `detail` is computed later,
    /// only for the prefix-filtered survivors.
    origin: Option<SmolStr>,
}

/// Resolve completion off the snapshot's cached parse when the db's tracked
/// buffer for `path` still matches `text`; otherwise re-parse. Falls back on
/// cancellation. The harvested index comes from the same snapshot.
pub(crate) fn completion_via_db(
    snapshot: &Analysis,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<CompletionResponse> {
    let line_index = LineIndex::new(text);
    let offset = TextSize::new(line_index.position_to_byte(position).min(text.len()) as u32);
    let index = snapshot.library_data().unwrap_or_default();
    let remote = snapshot.remote_exports().unwrap_or_default();
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        Some(completions_from_node(&root, offset, &index, &remote))
    }));
    match cached {
        Ok(Some(resp)) => resp,
        Ok(None) | Err(_) => {
            let root = parse(text).cst;
            completions_from_node(&root, offset, &index, &remote)
        }
    }
}

/// Build completions at byte `offset`. Pure (parses `text` itself) so it is
/// unit-testable; the LSP read path uses [`completion_via_db`].
pub fn compute_completions(
    text: &str,
    offset: usize,
    indexed: &IndexedProvider,
) -> Option<CompletionResponse> {
    let root = parse(text).cst;
    let offset = TextSize::new(offset.min(text.len()) as u32);
    completions_from_node(&root, offset, indexed, &RemoteExports::new())
}

/// Attach documentation + signature to a resolved completion item, using the
/// identity stashed in its `data`. Returns the item unchanged when there is
/// nothing to attach (a local, an unindexed symbol, or malformed `data`).
pub fn resolve_completion(mut item: CompletionItem, indexed: &IndexedProvider) -> CompletionItem {
    let Some(data) = item
        .data
        .clone()
        .and_then(|v| serde_json::from_value::<CompletionData>(v).ok())
    else {
        return item;
    };
    let resolved = match &data {
        CompletionData::Member { package, name } => indexed
            .lookup(package, name)
            .map(|entry| (package.clone(), entry)),
        CompletionData::Bare { name } => base_package_of(name)
            .and_then(|pkg| indexed.lookup(pkg, name).map(|entry| (pkg.clone(), entry))),
        CompletionData::Local | CompletionData::Field => None,
    };
    if let Some((package, entry)) = resolved {
        item.documentation = Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: render_hover_markdown(&package, entry),
        }));
        item.detail = signature_of(entry);
    }
    item
}

/// Build completions off an already-parsed CST.
pub(crate) fn completions_from_node(
    root: &SyntaxNode,
    offset: TextSize,
    indexed: &IndexedProvider,
    remote: &RemoteExports,
) -> Option<CompletionResponse> {
    match classify_context(root, offset) {
        CompletionContext::None => None,
        CompletionContext::Member {
            package,
            internal,
            prefix,
        } => Some(build_response(
            member_candidates(indexed, remote, &package, internal),
            &prefix,
            true,
            indexed,
        )),
        CompletionContext::Field {
            receiver,
            at,
            prefix,
        } => Some(build_response(
            field_candidates(root, &receiver, at),
            &prefix,
            true,
            indexed,
        )),
        CompletionContext::Bare { prefix, offset } => Some(build_response(
            bare_candidates(root, offset, indexed, remote),
            &prefix,
            false,
            indexed,
        )),
    }
}

/// Classify what the cursor at `offset` is positioned to complete.
fn classify_context(root: &SyntaxNode, offset: TextSize) -> CompletionContext {
    if in_string_or_comment(root, offset) {
        return CompletionContext::None;
    }
    if let Some((package, internal, prefix)) = member_context(root, offset) {
        return CompletionContext::Member {
            package,
            internal,
            prefix,
        };
    }
    if let Some((receiver, at, prefix)) = field_context(root, offset) {
        return CompletionContext::Field {
            receiver,
            at,
            prefix,
        };
    }
    bare_context(root, offset)
}

/// True when the cursor sits inside a string or comment (no completion there).
fn in_string_or_comment(root: &SyntaxNode, offset: TextSize) -> bool {
    let bad = |k: SyntaxKind| matches!(k, SyntaxKind::STRING | SyntaxKind::COMMENT);
    match root.token_at_offset(offset) {
        TokenAtOffset::None => false,
        TokenAtOffset::Single(t) => bad(t.kind()),
        // A boundary: the cursor is typing into the left token. Only comments
        // (which run to end of line) keep us "inside" at their trailing edge.
        TokenAtOffset::Between(left, _) => left.kind() == SyntaxKind::COMMENT,
    }
}

/// Detect a `pkg::`/`pkg:::` member-completion context: either a partial RHS
/// name, or a just-typed operator with nothing after it.
fn member_context(root: &SyntaxNode, offset: TextSize) -> Option<(SmolStr, bool, String)> {
    // Partial name: the cursor is on the RHS name of a `pkg::name` access.
    if let Some(token) = pick_name_token(root, offset) {
        for ancestor in token.parent_ancestors() {
            if let Some(access) = BinaryExpr::cast(ancestor).and_then(|b| b.namespace_access())
                && access.name_token == token
            {
                return Some((
                    access.package,
                    access.internal,
                    prefix_in_token(&token, offset),
                ));
            }
        }
    }
    // Just-typed `pkg::` with no RHS yet: recover via a token-level left-scan.
    recover_namespace_at(root, offset).map(|(pkg, internal)| (pkg, internal, String::new()))
}

/// Recover the package + operator kind when the cursor follows a `::`/`:::`
/// that has no right-hand side yet (so no clean `BINARY_EXPR` formed).
fn recover_namespace_at(root: &SyntaxNode, offset: TextSize) -> Option<(SmolStr, bool)> {
    let left = match root.token_at_offset(offset) {
        TokenAtOffset::Single(t) => Some(t),
        TokenAtOffset::Between(l, _) => Some(l),
        TokenAtOffset::None => None,
    }?;
    let op = skip_trivia_left(left)?;
    let internal = match op.kind() {
        SyntaxKind::COLON2 => false,
        SyntaxKind::COLON3 => true,
        _ => return None,
    };
    let pkg = prev_non_trivia(&op)?;
    if !matches!(pkg.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) {
        return None;
    }
    Some((token_text_unquoted(&pkg), internal))
}

/// Detect a `receiver$`/`receiver@` field-completion context: either a partial
/// RHS field name, or a just-typed operator with nothing after it. Mirrors
/// [`member_context`]. Returns the normalized receiver text, whether the
/// operator is `@` (S4 slot) versus `$`, and the typed prefix.
fn field_context(root: &SyntaxNode, offset: TextSize) -> Option<(String, bool, String)> {
    // Partial name: the cursor is on the RHS field of a `receiver$field` access.
    if let Some(token) = pick_name_token(root, offset) {
        for ancestor in token.parent_ancestors() {
            let Some(binary) = BinaryExpr::cast(ancestor) else {
                continue;
            };
            let Some(at) = field_op(binary.op_kind()) else {
                continue;
            };
            if matches!(binary.rhs(), Some(SyntaxElement::Token(rhs)) if rhs == token)
                && let Some(lhs) = binary.lhs()
            {
                return Some((
                    normalize_receiver(&lhs),
                    at,
                    prefix_in_token(&token, offset),
                ));
            }
        }
    }
    // Just-typed `receiver$` with no RHS yet: recover via a token-level left-scan.
    recover_field_at(root, offset).map(|(recv, at)| (recv, at, String::new()))
}

/// `Some(true)` for `@`, `Some(false)` for `$`, `None` otherwise.
fn field_op(kind: Option<SyntaxKind>) -> Option<bool> {
    match kind {
        Some(SyntaxKind::DOLLAR) => Some(false),
        Some(SyntaxKind::AT) => Some(true),
        _ => None,
    }
}

/// Recover the receiver + operator kind when the cursor follows a `$`/`@` that
/// has no right-hand side yet (so no clean `BINARY_EXPR` formed — it lands in an
/// `ERROR` node). The receiver is the immediately preceding name token, so a
/// chained `a$b$` recovers as `b`, not `a$b` (a documented v1 limitation).
fn recover_field_at(root: &SyntaxNode, offset: TextSize) -> Option<(String, bool)> {
    let left = match root.token_at_offset(offset) {
        TokenAtOffset::Single(t) => Some(t),
        TokenAtOffset::Between(l, _) => Some(l),
        TokenAtOffset::None => None,
    }?;
    let op = skip_trivia_left(left)?;
    let at = field_op(Some(op.kind()))?;
    let recv = prev_non_trivia(&op)?;
    if !matches!(recv.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) {
        return None;
    }
    Some((token_text_unquoted(&recv).to_string(), at))
}

/// Whitespace-normalized source text of a receiver operand, used as the key that
/// ties a completion request to the `$`/`@` accesses that share it.
fn normalize_receiver(el: &SyntaxElement) -> String {
    let text = match el {
        SyntaxElement::Node(n) => n.text().to_string(),
        SyntaxElement::Token(t) => t.text().to_string(),
    };
    text.split_whitespace().collect()
}

/// Bare-name context, unless the cursor is on the package operand of a
/// `pkg::name` access (we never complete package names).
fn bare_context(root: &SyntaxNode, offset: TextSize) -> CompletionContext {
    if let Some(token) = pick_name_token(root, offset) {
        for ancestor in token.parent_ancestors() {
            if ancestor.kind() == SyntaxKind::BINARY_EXPR
                && let Some(access) = BinaryExpr::cast(ancestor).and_then(|b| b.namespace_access())
                && access.package_token == token
            {
                return CompletionContext::None;
            }
        }
        return CompletionContext::Bare {
            prefix: prefix_in_token(&token, offset),
            offset,
        };
    }
    // No name under the cursor (blank space, after an operator): allow an
    // explicit, empty-prefix invocation.
    CompletionContext::Bare {
        prefix: String::new(),
        offset,
    }
}

/// Exported (or, for `:::`, all) symbols of `package`. Falls back through the
/// names-only tiers when the package isn't locally harvested: the remote sidecar
/// first, then the baked-in bundled list. The names-only tiers carry only
/// exports, so `:::` cannot surface internals there.
fn member_candidates(
    indexed: &IndexedProvider,
    remote: &RemoteExports,
    package: &str,
    internal: bool,
) -> Vec<Candidate> {
    if let Some(pkg) = indexed.package(package) {
        return pkg
            .symbols
            .iter()
            .filter(|s| internal || s.exported)
            .map(|s| Candidate {
                label: s.name.to_string(),
                kind: kind_of(s.kind),
                sort_group: 3,
                data: CompletionData::Member {
                    package: SmolStr::new(package),
                    name: s.name.clone(),
                },
                origin: Some(SmolStr::new(package)),
            })
            .collect();
    }
    let names = remote
        .package_exports(package)
        .map(|it| it.collect::<Vec<_>>())
        .or_else(|| bundled_exports(package).map(|it| it.collect::<Vec<_>>()));
    match names {
        Some(names) => names
            .into_iter()
            .map(|n| Candidate {
                label: n.to_string(),
                kind: CompletionItemKind::FUNCTION,
                sort_group: 3,
                data: CompletionData::Member {
                    package: SmolStr::new(package),
                    name: n.clone(),
                },
                origin: Some(SmolStr::new(package)),
            })
            .collect(),
        None => Vec::new(),
    }
}

/// `$`/`@` field candidates for `receiver`, gathered statically (arity does not
/// evaluate). Harvests field names used with the same operator on the same
/// receiver anywhere in the file, and — for `$` only — infers the named fields
/// of a local `list()`/`data.frame()`-family construction bound to `receiver`.
fn field_candidates(root: &SyntaxNode, receiver: &str, at: bool) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    // 1. Harvest field names used on the same receiver elsewhere in the file.
    for node in root.descendants() {
        let Some(binary) = BinaryExpr::cast(node) else {
            continue;
        };
        if field_op(binary.op_kind()) != Some(at) {
            continue;
        }
        let Some(lhs) = binary.lhs() else { continue };
        if normalize_receiver(&lhs) != receiver {
            continue;
        }
        if let Some(SyntaxElement::Token(name)) = binary.rhs()
            && matches!(name.kind(), SyntaxKind::IDENT | SyntaxKind::STRING)
        {
            out.push(field_candidate(token_text_unquoted(&name)));
        }
    }

    // 2. Infer fields from a local record construction bound to the receiver.
    //    S4 slot inference (`new()`/`setClass`) is a follow-up, so `@` harvests only.
    if !at {
        out.extend(constructed_fields(root, receiver));
    }

    out
}

/// The named arguments of a `list()`/`data.frame()`/`tibble()`/`data.table()`
/// call assigned to `receiver` (`receiver <- data.frame(x = …, y = …)`).
fn constructed_fields(root: &SyntaxNode, receiver: &str) -> Vec<Candidate> {
    const CONSTRUCTORS: [&str; 4] = ["list", "data.frame", "tibble", "data.table"];
    let mut out = Vec::new();
    for node in root.descendants() {
        let Some(assign) = AssignmentExpr::cast(node) else {
            continue;
        };
        if assign.target_name().as_deref() != Some(receiver) {
            continue;
        }
        let Some(SyntaxElement::Node(value)) = assign.value_element() else {
            continue;
        };
        let Some(call) = CallExpr::cast(value) else {
            continue;
        };
        if !CONSTRUCTORS.contains(&call.callee_name().as_deref().unwrap_or_default()) {
            continue;
        }
        if let Some(arg_list) = call.arg_list() {
            out.extend(
                arg_list
                    .args()
                    .filter_map(|a| a.name())
                    .map(field_candidate),
            );
        }
    }
    out
}

fn field_candidate(name: SmolStr) -> Candidate {
    Candidate {
        label: name.to_string(),
        kind: CompletionItemKind::FIELD,
        sort_group: 0,
        data: CompletionData::Field,
        origin: None,
    }
}

/// Bare-name candidates: scope-visible locals, attached-package exports, and
/// base-R default-package names.
fn bare_candidates(
    root: &SyntaxNode,
    offset: TextSize,
    indexed: &IndexedProvider,
    remote: &RemoteExports,
) -> Vec<Candidate> {
    let model = SemanticModel::build(root);
    let mut out: Vec<Candidate> = Vec::new();

    // 1. Scope-visible locals.
    for (name, _kind) in model.names_in_scope_at(offset) {
        out.push(Candidate {
            label: name.to_string(),
            kind: CompletionItemKind::VARIABLE,
            sort_group: 0,
            data: CompletionData::Local,
            origin: Some(SmolStr::new_static("local")),
        });
    }

    // 2. Exports of `library()`-attached packages.
    for pkg in model.loaded_packages() {
        if let Some(idx) = indexed.package(&pkg.name) {
            out.extend(
                idx.symbols
                    .iter()
                    .filter(|s| s.exported)
                    .map(|s| Candidate {
                        label: s.name.to_string(),
                        kind: kind_of(s.kind),
                        sort_group: 1,
                        data: CompletionData::Member {
                            package: pkg.name.clone(),
                            name: s.name.clone(),
                        },
                        origin: Some(pkg.name.clone()),
                    }),
            );
        } else if let Some(names) = remote
            .package_exports(&pkg.name)
            .map(|it| it.collect::<Vec<_>>())
            .or_else(|| bundled_exports(&pkg.name).map(|it| it.collect::<Vec<_>>()))
        {
            out.extend(names.into_iter().map(|n| Candidate {
                label: n.to_string(),
                kind: CompletionItemKind::FUNCTION,
                sort_group: 1,
                data: CompletionData::Member {
                    package: pkg.name.clone(),
                    name: n.clone(),
                },
                origin: Some(pkg.name.clone()),
            }));
        }
    }

    // 3. Base-R default-package names.
    out.extend(base_names().map(|name| Candidate {
        label: name.to_string(),
        kind: CompletionItemKind::FUNCTION,
        sort_group: 2,
        data: CompletionData::Bare { name: name.clone() },
        origin: base_package_of(name).cloned(),
    }));

    out
}

/// Prefix-filter, dedup (lowest `sort_group` wins per label), and assemble the
/// response. Bare lists are marked incomplete (filtered from a large universe)
/// so the client re-queries as typing continues. Label details (origin +
/// signature) are attached here, after filtering, so the signature lookups touch
/// only the surviving set — never the full base-R universe.
fn build_response(
    mut cands: Vec<Candidate>,
    prefix: &str,
    member: bool,
    indexed: &IndexedProvider,
) -> CompletionResponse {
    if !prefix.is_empty() {
        cands.retain(|c| c.label.starts_with(prefix));
    }
    // Sort by label then group so the first of each label run is the lowest
    // group; dedup keeps that one (e.g. a local masks the base name once).
    cands.sort_by(|a, b| a.label.cmp(&b.label).then(a.sort_group.cmp(&b.sort_group)));
    cands.dedup_by(|a, b| a.label == b.label);
    let items = cands
        .into_iter()
        .map(|c| {
            let detail = signature_detail(&c.data, indexed);
            let description = c.origin.as_deref().map(str::to_string);
            let label_details =
                (detail.is_some() || description.is_some()).then_some(CompletionItemLabelDetails {
                    detail,
                    description,
                });
            CompletionItem {
                sort_text: Some(format!("{}{}", c.sort_group, c.label)),
                filter_text: Some(c.label.clone()),
                kind: Some(c.kind),
                data: serde_json::to_value(c.data).ok(),
                label_details,
                label: c.label,
                ..Default::default()
            }
        })
        .collect();
    CompletionResponse::List(CompletionList {
        is_incomplete: !member,
        items,
    })
}

/// The parenthesized parameter list (`(.cols, .fns)`) for a completion that
/// resolves to an indexed function entry, shown inline after the label. `None`
/// for a local, a `$`/`@` field, or a symbol with no harvested formals.
fn signature_detail(data: &CompletionData, indexed: &IndexedProvider) -> Option<String> {
    let entry = match data {
        CompletionData::Member { package, name } => indexed.lookup(package, name)?,
        CompletionData::Bare { name } => indexed.lookup(base_package_of(name)?, name)?,
        CompletionData::Local | CompletionData::Field => return None,
    };
    // `signature_of` yields `name(args)`; keep just `(args)` so it does not
    // duplicate the label.
    let sig = signature_of(entry)?;
    let paren = sig.find('(')?;
    Some(sig[paren..].to_string())
}

fn kind_of(kind: SymbolKind) -> CompletionItemKind {
    match kind {
        SymbolKind::Function => CompletionItemKind::FUNCTION,
        SymbolKind::Data => CompletionItemKind::VALUE,
        SymbolKind::Other => CompletionItemKind::FIELD,
    }
}

/// The token text up to `offset` — the prefix the user has typed so far.
fn prefix_in_token(token: &SyntaxToken<RLanguage>, offset: TextSize) -> String {
    let start = token.text_range().start();
    let rel = offset.checked_sub(start).map_or(0, u32::from) as usize;
    let text = token.text();
    text.get(..rel.min(text.len())).unwrap_or(text).to_string()
}

/// The unquoted name a token denotes (raw `IDENT`, or quoted `STRING` contents).
fn token_text_unquoted(token: &SyntaxToken<RLanguage>) -> SmolStr {
    if token.kind() == SyntaxKind::STRING {
        let text = token.text();
        if text.len() >= 2 {
            return SmolStr::new(&text[1..text.len() - 1]);
        }
    }
    SmolStr::new(token.text())
}

fn is_trivia_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

/// `token`, or the nearest preceding non-trivia token in document order.
fn skip_trivia_left(token: SyntaxToken<RLanguage>) -> Option<SyntaxToken<RLanguage>> {
    let mut cur = Some(token);
    while let Some(t) = cur {
        if !is_trivia_kind(t.kind()) {
            return Some(t);
        }
        cur = t.prev_token();
    }
    None
}

/// The nearest non-trivia token strictly before `token` in document order.
fn prev_non_trivia(token: &SyntaxToken<RLanguage>) -> Option<SyntaxToken<RLanguage>> {
    let mut cur = token.prev_token();
    while let Some(t) = cur {
        if !is_trivia_kind(t.kind()) {
            return Some(t);
        }
        cur = t.prev_token();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};

    fn items(resp: CompletionResponse) -> Vec<CompletionItem> {
        match resp {
            CompletionResponse::Array(v) => v,
            CompletionResponse::List(l) => l.items,
        }
    }

    fn labels(resp: CompletionResponse) -> Vec<String> {
        items(resp).into_iter().map(|i| i.label).collect()
    }

    fn at_end(src: &str, needle: &str) -> usize {
        src.find(needle).expect("needle present") + needle.len()
    }

    fn provider_with_unexported() -> IndexedProvider {
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "pkg".into(),
            version: "1.0".into(),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: vec![
                SymbolEntry {
                    name: "pub_fn".into(),
                    kind: SymbolKind::Function,
                    exported: true,
                    formals: None,
                    help: None,
                },
                SymbolEntry {
                    name: "priv_fn".into(),
                    kind: SymbolKind::Function,
                    exported: false,
                    formals: None,
                    help: None,
                },
            ],
        };
        IndexedProvider::from_indices([idx])
    }

    #[test]
    fn member_completion_after_pkg_colons() {
        // `dplyr::` with nothing after: the parse-recovery left-scan finds the
        // package and lists its exports.
        let src = "dplyr::\n";
        let got = labels(compute_completions(src, at_end(src, "::"), &indexed_dplyr()).unwrap());
        assert!(got.contains(&"across".to_string()), "{got:?}");
    }

    #[test]
    fn member_completion_with_partial_prefix() {
        let p = indexed_dplyr();
        let hit = "dplyr::acr\n";
        assert!(
            labels(compute_completions(hit, at_end(hit, "acr"), &p).unwrap())
                .contains(&"across".to_string())
        );
        let miss = "dplyr::zzz\n";
        assert!(labels(compute_completions(miss, at_end(miss, "zzz"), &p).unwrap()).is_empty());
    }

    #[test]
    fn member_internal_includes_unexported() {
        let p = provider_with_unexported();
        let public = labels(compute_completions("pkg::\n", at_end("pkg::\n", "::"), &p).unwrap());
        assert!(public.contains(&"pub_fn".to_string()));
        assert!(!public.contains(&"priv_fn".to_string()), "{public:?}");
        let internal =
            labels(compute_completions("pkg:::\n", at_end("pkg:::\n", ":::"), &p).unwrap());
        assert!(internal.contains(&"pub_fn".to_string()));
        assert!(internal.contains(&"priv_fn".to_string()), "{internal:?}");
    }

    #[test]
    fn member_falls_back_to_bundled() {
        // data.table isn't harvested into `indexed`, so the bundled names-only
        // list backs member completion.
        let src = "data.table::\n";
        let got =
            labels(compute_completions(src, at_end(src, "::"), &IndexedProvider::empty()).unwrap());
        assert!(got.contains(&"fread".to_string()), "{got:?}");
    }

    fn remote(pkg: &str, names: &[&str]) -> RemoteExports {
        let mut r = RemoteExports::new();
        r.insert_package(pkg, names.iter().map(|n| SmolStr::new(*n)));
        r
    }

    fn labels_with_remote(src: &str, needle: &str, remote: &RemoteExports) -> Vec<String> {
        let root = parse(src).cst;
        let offset = TextSize::new(at_end(src, needle) as u32);
        labels(completions_from_node(&root, offset, &IndexedProvider::empty(), remote).unwrap())
    }

    #[test]
    fn member_uses_remote_for_uninstalled_unbundled_package() {
        // `tinytable` is neither harvested nor bundled; the remote sidecar backs
        // `pkg::` member completion.
        let got = labels_with_remote(
            "tinytable::\n",
            "::",
            &remote("tinytable", &["tt", "theme_tt"]),
        );
        assert!(got.contains(&"tt".to_string()), "{got:?}");
        assert!(got.contains(&"theme_tt".to_string()), "{got:?}");
    }

    #[test]
    fn bare_uses_remote_for_attached_uninstalled_package() {
        let got = labels_with_remote(
            "library(tinytable)\ntt\n",
            "\ntt",
            &remote("tinytable", &["tt"]),
        );
        assert!(got.contains(&"tt".to_string()), "{got:?}");
    }

    #[test]
    fn bare_prefix_includes_local_and_base() {
        // `v` matches the local `value` (group 0) and base names like `vector`
        // (group 2); the local sorts ahead of base.
        let src = "value <- 1\nv\n";
        let off = src.rfind('v').unwrap() + 1;
        let its = items(compute_completions(src, off, &IndexedProvider::empty()).unwrap());
        let value = its
            .iter()
            .find(|i| i.label == "value")
            .expect("local value");
        assert_eq!(value.kind, Some(CompletionItemKind::VARIABLE));
        assert!(value.sort_text.as_deref().unwrap().starts_with('0'));
        assert!(its.iter().any(|i| i.label == "vector"), "a base name");
    }

    #[test]
    fn bare_local_masks_base_duplicate() {
        // A local named `mean` appears once, attributed to the local (group 0),
        // not the base function.
        let src = "mean <- 1\nmea\n";
        let off = at_end(src, "mea");
        let its = items(compute_completions(src, off, &IndexedProvider::empty()).unwrap());
        let means: Vec<_> = its.iter().filter(|i| i.label == "mean").collect();
        assert_eq!(means.len(), 1, "one `mean`: {its:?}");
        assert!(means[0].sort_text.as_deref().unwrap().starts_with('0'));
    }

    #[test]
    fn bare_includes_attached_export() {
        let src = "library(dplyr)\nacr\n";
        let got = labels(compute_completions(src, at_end(src, "acr"), &indexed_dplyr()).unwrap());
        assert!(got.contains(&"across".to_string()), "{got:?}");
    }

    #[test]
    fn locals_respect_scope() {
        // `a` (param of f) completes inside f but not g; `b` only inside g.
        let src = "f <- function(a) {\n  \n}\ng <- function(b) {\n  b\n}\n";
        let off_f = src.find("  \n").unwrap() + 2;
        let in_f = labels(compute_completions(src, off_f, &IndexedProvider::empty()).unwrap());
        assert!(in_f.contains(&"a".to_string()), "f sees a: {in_f:?}");
        assert!(!in_f.contains(&"b".to_string()), "f hides b: {in_f:?}");
    }

    #[test]
    fn no_completion_in_string() {
        let src = "x <- \"dpl\"\n";
        let off = src.find("dpl").unwrap() + 1;
        assert!(compute_completions(src, off, &documented_dplyr()).is_none());
    }

    #[test]
    fn no_completion_in_comment() {
        let src = "# dplyr::acr\n";
        assert!(compute_completions(src, at_end(src, "acr"), &indexed_dplyr()).is_none());
    }

    #[test]
    fn resolve_attaches_docs() {
        let item = CompletionItem {
            label: "across".into(),
            data: serde_json::to_value(CompletionData::Member {
                package: "dplyr".into(),
                name: "across".into(),
            })
            .ok(),
            ..Default::default()
        };
        let resolved = resolve_completion(item, &documented_dplyr());
        let doc = match resolved.documentation {
            Some(Documentation::MarkupContent(m)) => m.value,
            other => panic!("expected markdown, got {other:?}"),
        };
        assert!(doc.contains("dplyr::across"), "{doc}");
        assert_eq!(resolved.detail.as_deref(), Some("across(.cols, .fns)"));
    }

    #[test]
    fn resolve_local_unchanged() {
        let item = CompletionItem {
            label: "x".into(),
            data: serde_json::to_value(CompletionData::Local).ok(),
            ..Default::default()
        };
        let resolved = resolve_completion(item, &documented_dplyr());
        assert!(resolved.documentation.is_none());
        assert!(resolved.detail.is_none());
    }

    // --- `$`/`@` member (field) completion --------------------------------

    fn dollar_labels(src: &str) -> Vec<String> {
        // Cursor sits right after the last `$`/`@` occurrence's operator.
        let off = src.rfind(['$', '@']).expect("dollar/at present") + 1;
        labels(compute_completions(src, off, &IndexedProvider::empty()).unwrap())
    }

    #[test]
    fn no_bare_leak_after_dollar() {
        // After `df$`, only fields are completed — never locals, base, or pkg names.
        let src = "mean_val <- 1\ndf$\n";
        let got = dollar_labels(src);
        assert!(
            !got.contains(&"mean_val".to_string()),
            "no local leak: {got:?}"
        );
        assert!(
            !got.contains(&"vector".to_string()),
            "no base leak: {got:?}"
        );
    }

    #[test]
    fn field_harvests_dollar_usage() {
        let src = "df$foo <- 1\ndf$bar <- 2\ndf$\n";
        let got = dollar_labels(src);
        assert!(got.contains(&"foo".to_string()), "{got:?}");
        assert!(got.contains(&"bar".to_string()), "{got:?}");
    }

    #[test]
    fn field_infers_from_data_frame() {
        let src = "df <- data.frame(x = 1, y = 2)\ndf$\n";
        let got = dollar_labels(src);
        assert!(got.contains(&"x".to_string()), "{got:?}");
        assert!(got.contains(&"y".to_string()), "{got:?}");
    }

    #[test]
    fn field_infers_from_list_construction() {
        let src = "cfg <- list(alpha = 1, beta = 2)\ncfg$\n";
        let got = dollar_labels(src);
        assert!(got.contains(&"alpha".to_string()), "{got:?}");
        assert!(got.contains(&"beta".to_string()), "{got:?}");
    }

    #[test]
    fn field_respects_receiver() {
        // A different receiver's fields must not leak into `df$` completion.
        let src = "df$foo <- 1\nother$zzz <- 2\ndf$\n";
        let got = dollar_labels(src);
        assert!(got.contains(&"foo".to_string()), "{got:?}");
        assert!(
            !got.contains(&"zzz".to_string()),
            "no cross-receiver: {got:?}"
        );
    }

    #[test]
    fn field_at_slot_harvest() {
        let src = "obj@alpha <- 1\nobj@\n";
        let got = dollar_labels(src);
        assert!(got.contains(&"alpha".to_string()), "{got:?}");
    }

    #[test]
    fn field_partial_prefix_filters() {
        let src = "df$foo <- 1\ndf$bar <- 2\ndf$f\n";
        let off = at_end(src, "df$f");
        let got = labels(compute_completions(src, off, &IndexedProvider::empty()).unwrap());
        assert!(got.contains(&"foo".to_string()), "{got:?}");
        assert!(
            !got.contains(&"bar".to_string()),
            "prefix filters bar: {got:?}"
        );
    }

    // --- label details ----------------------------------------------------

    fn label_details(item: &CompletionItem) -> (Option<&str>, Option<&str>) {
        let d = item.label_details.as_ref().expect("label details present");
        (d.detail.as_deref(), d.description.as_deref())
    }

    #[test]
    fn member_label_details_show_package_and_signature() {
        let src = "dplyr::acr\n";
        let its = items(compute_completions(src, at_end(src, "acr"), &documented_dplyr()).unwrap());
        let across = its.iter().find(|i| i.label == "across").expect("across");
        assert_eq!(
            label_details(across),
            (Some("(.cols, .fns)"), Some("dplyr"))
        );
    }

    #[test]
    fn bare_base_label_details_show_base_origin() {
        let src = "vec\n";
        let its =
            items(compute_completions(src, at_end(src, "vec"), &IndexedProvider::empty()).unwrap());
        let v = its
            .iter()
            .find(|i| i.label == "vector")
            .expect("base vector");
        assert_eq!(label_details(v).1, Some("base"));
    }

    #[test]
    fn local_label_details_show_local_origin() {
        let src = "value <- 1\nv\n";
        let its =
            items(compute_completions(src, at_end(src, "\nv"), &IndexedProvider::empty()).unwrap());
        let value = its
            .iter()
            .find(|i| i.label == "value")
            .expect("local value");
        assert_eq!(label_details(value).1, Some("local"));
    }
}
