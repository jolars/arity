//! Cross-edit node handles: `NodePtr` round-trips, cross-edit re-resolution via
//! range mapping, and serde of the LSP `data`-ready handle.

use ravel::ast::{AssignmentExpr, AstNode, CallExpr, FunctionExpr};
use ravel::parser::{Edit, map_range_through_edit, parse};
use ravel::syntax::{NodePtr, SyntaxKind, SyntaxNode};

fn root(src: &str) -> SyntaxNode {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    parsed.cst
}

fn first<N: AstNode<Language = ravel::syntax::RLanguage>>(root: &SyntaxNode) -> N {
    root.descendants().find_map(N::cast).expect("a node")
}

#[test]
fn round_trips_against_same_revision_tree() {
    let cases = [
        "f <- function(x) x + 1\n",
        "if (a) b else c\n",
        "dplyr::filter(df, x > 1)\n",
    ];
    for src in cases {
        let root = root(src);
        for node in root.descendants() {
            let ptr = NodePtr::from_node(&node);
            assert_eq!(
                ptr.try_to_node(&root).as_ref(),
                Some(&node),
                "round-trip failed for {:?} in {src:?}",
                node.kind()
            );
            assert_eq!(ptr.kind(), node.kind());
            assert_eq!(ptr.text_range(), node.text_range());
        }
    }
}

#[test]
fn resolves_after_an_edit_before_the_node() {
    // Take a handle to the `function` node, then prepend a comment line.
    let old = "f <- function(x) x + 1\n";
    let func: FunctionExpr = first(&root(old));
    let ptr = NodePtr::from_node(func.syntax());

    let edit = Edit {
        range: 0..0,
        insert: "# header\n".to_string(),
    };
    let new = edit.apply(old);
    let new_root = root(&new);

    let mapped = map_range_through_edit(ptr.text_range(), &edit).expect("range maps");
    let resolved = ptr
        .with_range(mapped)
        .try_to_node(&new_root)
        .expect("resolves");
    assert_eq!(resolved.kind(), SyntaxKind::FUNCTION_EXPR);
    assert_eq!(resolved.text(), func.syntax().text());
}

#[test]
fn invalidates_when_the_node_itself_is_edited() {
    let old = "foo <- 1\n";
    let assign: AssignmentExpr = first(&root(old));
    let target = assign.target_name_token().expect("target token");
    let ptr = NodePtr::from_node(assign.syntax());

    // Replace `foo` with `barbar`: the edit lands inside the captured node.
    let span = target.text_range();
    let edit = Edit {
        range: usize::from(span.start())..usize::from(span.end()),
        insert: "barbar".to_string(),
    };
    assert_eq!(map_range_through_edit(ptr.text_range(), &edit), None);
}

#[test]
fn serde_round_trip_preserves_resolution() {
    let src = "g <- function() do_thing()\n";
    let root = root(src);
    let call: CallExpr = first(&root);
    let ptr = NodePtr::from_node(call.syntax());

    let json = serde_json::to_string(&ptr).expect("serialize");
    let back: NodePtr = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, ptr);
    assert_eq!(
        back.try_to_node(&root).map(|n| n.kind()),
        Some(SyntaxKind::CALL_EXPR)
    );
}

#[test]
fn deserialize_rejects_unknown_kind() {
    // The serde form names the kind; an unknown name must not silently decode.
    let blob = r#"{"kind":"NOT_A_REAL_KIND","start":0,"len":1}"#;
    assert!(serde_json::from_str::<NodePtr>(blob).is_err());
}
