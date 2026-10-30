//! WP-U2 acceptance (ui-protocol §4.7): `apply(old, diff(old, new)) == new`
//! over 10 000 proptest cases, the fallback-to-replace rule (§4.7 rules 1 and
//! 4), op-level unit cases, and the coverage matrix of the ratatui-generated
//! `layout_vectors.json` (§12.4 / WP-U2 row).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use proptest::prelude::*;
use serde_json::{Value as Json, json};

use oino_ui::layout::{Constraint, Direction, Flex, Margin};
use oino_ui::node::Node;
use oino_ui::patch::{PatchError, apply, diff};
use oino_ui::prelude::*;

// ---------------------------------------------------------------- helpers

/// Parses a node JSON literal (tolerant parse path, §4.8).
fn node(json: Json) -> Node {
    serde_json::from_value(json).expect("node json parses")
}

/// Round-trips `old → diff → apply` and returns the ops for assertions.
fn roundtrip(old: &Node, new: &Node) -> Vec<NodeOp> {
    let ops = diff(old, new);
    let applied = apply(old, &ops).expect("ops apply");
    assert_eq!(applied, *new, "apply(old, diff(old, new)) must equal new");
    ops
}

/// Minimal valid props for the leaf kinds the proptest generator emits.
fn leaf_table() -> [(&'static str, Json); 12] {
    [
        ("Block", json!({})),
        ("Empty", json!({})),
        ("Spinner", json!({})),
        ("Gauge", json!({})),
        ("Input", json!({})),
        ("Diff", json!({})),
        ("Paragraph", json!({"text": "hi"})),
        ("Markdown", json!({"source": "# hi"})),
        ("Code", json!({"source": "let x = 1;"})),
        ("Toggle", json!({"label": "t", "value": true})),
        ("Button", json!({"label": "go", "action": "go"})),
        ("LineGauge", json!({"ratio": 0.5})),
    ]
}

/// Builds one leaf node with the optional common props.
fn leaf_from(variant: u8, key: Option<String>, style: bool, fallback: Option<Json>) -> Json {
    let (tag, base) = &leaf_table()[variant as usize % 12];
    let mut obj = base.clone();
    let map = obj.as_object_mut().expect("leaf literal is an object");
    map.insert("t".into(), json!(tag));
    if let Some(key) = key {
        map.insert("key".into(), json!(key));
    }
    if style {
        map.insert("style".into(), json!({"fg": "accent"}));
    }
    if let Some(fallback) = fallback {
        map.insert("fallback".into(), fallback);
    }
    obj
}

/// One child spec: leaf variant + key/style/fallback choices.
fn child_spec() -> impl Strategy<Value = (u8, bool, bool, bool)> {
    (0u8..12, any::<bool>(), any::<bool>(), any::<bool>())
}

/// A leaf with per-position unique key `k<i>` when requested.
fn keyed_leaf(index: usize, spec: (u8, bool, bool, bool)) -> Json {
    let (variant, key, style, fallback) = spec;
    leaf_from(
        variant,
        key.then(|| format!("k{index}")),
        style,
        fallback.then(|| leaf_from((variant + 1) % 12, None, false, None)),
    )
}

/// Layout children: 0..4 keyed leaves.
fn children_json() -> impl Strategy<Value = Vec<Json>> {
    proptest::collection::vec(child_spec(), 0..4).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(|(index, spec)| keyed_leaf(index, spec))
            .collect()
    })
}

/// `List.items`: 0..4 text or keyed-full items.
fn items_json() -> impl Strategy<Value = Vec<Json>> {
    proptest::collection::vec((any::<bool>(), any::<bool>()), 0..4).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(|(index, (full, key))| {
                if full {
                    json!({"key": key.then(|| format!("k{index}")), "content": format!("item{index}")})
                } else {
                    json!(format!("item{index}"))
                }
            })
            .collect::<Vec<_>>()
    })
}

/// `Table.rows`: 0..4 keyed single-cell rows.
fn rows_json() -> impl Strategy<Value = Vec<Json>> {
    proptest::collection::vec((any::<bool>(), any::<bool>()), 0..4).prop_map(|specs| {
        specs
            .into_iter()
            .enumerate()
            .map(|(index, (key, _style))| {
                json!({"cells": [format!("c{index}")], "key": key.then(|| format!("k{index}"))})
            })
            .collect::<Vec<_>>()
    })
}

/// A standalone leaf (the fallback-able common-props sweep).
fn leaf_json() -> impl Strategy<Value = Json> {
    (0u8..12, any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(variant, key, style, fallback)| {
            leaf_from(
                variant,
                key.then(|| "k0".to_owned()),
                style,
                fallback.then(|| leaf_from((variant + 1) % 12, None, false, None)),
            )
        },
    )
}

/// A whole node tree: leaves plus one of each keyed/plain collection kind.
fn node_json() -> impl Strategy<Value = Json> {
    let children = children_json();
    let items = items_json();
    let lines = proptest::collection::vec(any::<bool>(), 0..4).prop_map(|flags| {
        flags
            .into_iter()
            .enumerate()
            .map(|(index, _flag)| json!(format!("line{index}")))
            .collect::<Vec<_>>()
    });
    let titles = proptest::collection::vec(any::<bool>(), 1..4).prop_map(|flags| {
        flags
            .into_iter()
            .enumerate()
            .map(|(index, _flag)| json!({"key": format!("k{index}"), "title": format!("t{index}")}))
            .collect::<Vec<_>>()
    });
    let panes = proptest::collection::vec(child_spec(), 0..3);
    let tree_nodes = proptest::collection::vec(any::<bool>(), 0..4).prop_map(|flags| {
        flags
            .into_iter()
            .enumerate()
            .map(|(index, _flag)| json!({"key": format!("k{index}"), "label": format!("n{index}")}))
            .collect::<Vec<_>>()
    });
    let datasets = proptest::collection::vec(any::<bool>(), 0..3).prop_map(|flags| {
        flags
            .into_iter()
            .enumerate()
            .map(|(index, _flag)| {
                let x = f64::from(u8::try_from(index).unwrap_or(0));
                json!({"data": [[x, x + 0.5]]})
            })
            .collect::<Vec<_>>()
    });
    let sparkline = proptest::collection::vec(any::<bool>(), 0..4).prop_map(|flags| {
        Json::Array(
            flags
                .into_iter()
                .enumerate()
                .map(|(index, filled)| {
                    if filled {
                        json!(index as u64 + 1)
                    } else {
                        Json::Null
                    }
                })
                .collect(),
        )
    });
    let bars = proptest::collection::vec(any::<bool>(), 0..3).prop_map(|flags| {
        vec![json!({
            "bars": flags.into_iter().enumerate()
                .map(|(index, _flag)| json!({"value": index as u64 + 1}))
                .collect::<Vec<_>>()
        })]
    });

    proptest::prop_oneof![
        4 => leaf_json(),
        2 => (children, any::<bool>(), any::<bool>()).prop_map(|(children, key, style)| {
            json!({"t": "Layout", "children": children, "key": key.then(|| "k0".to_owned()), "style": style.then(|| json!({"fg": "accent"}))})
        }),
        1 => items.prop_map(|items| json!({"t": "List", "items": items})),
        1 => rows_json().prop_map(|rows| json!({"t": "Table", "rows": rows, "widths": [10]})),
        1 => lines.prop_map(|lines| json!({"t": "Log", "lines": lines})),
        1 => (titles, panes).prop_map(|(titles, panes)| {
            let mut obj = json!({"t": "Tabs", "titles": titles});
            let map = obj.as_object_mut().expect("tabs is an object");
            let entries: serde_json::Map<String, Json> = panes
                .into_iter()
                .enumerate()
                .map(|(index, spec)| (format!("p{index}"), keyed_leaf(index, spec)))
                .collect();
            if !entries.is_empty() {
                map.insert("panes".into(), Json::Object(entries));
            }
            obj
        }),
        1 => tree_nodes.prop_map(|nodes| json!({"t": "Tree", "nodes": nodes})),
        1 => datasets.prop_map(|datasets| json!({"t": "Chart", "datasets": datasets})),
        1 => sparkline.prop_map(|data| json!({"t": "Sparkline", "data": data})),
        1 => bars.prop_map(|data| json!({"t": "BarChart", "data": data})),
    ]
    .boxed()
}

/// Deterministically perturbs a generated node (seeded LCG), so the proptest
/// also covers *related* old/new pairs (small ops), not just unrelated trees.
fn mutate(old: &Json, seed: u64) -> Json {
    let mut state = seed | 1;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    let mut new = old.clone();
    let Some(map) = new.as_object_mut() else {
        return new;
    };
    match next() % 8 {
        0 => {
            if next() % 2 == 0 {
                map.insert("style".into(), json!({"fg": "error", "mods": ["BOLD"]}));
            } else {
                map.remove("style");
            }
        }
        1 => {
            if map.contains_key("key") {
                map.remove("key");
            } else {
                map.insert("key".into(), json!("mut"));
            }
        }
        2 => {
            if let Some(children) = map.get_mut("children").and_then(Json::as_array_mut) {
                children.push(json!({"t": "Paragraph", "text": "appended"}));
            }
        }
        3 => {
            if let Some(children) = map.get_mut("children").and_then(Json::as_array_mut) {
                children.pop();
            }
        }
        4 => {
            if let Some(lines) = map.get_mut("lines").and_then(Json::as_array_mut) {
                lines.push(json!("appended"));
            }
        }
        5 => {
            if let Some(items) = map.get_mut("items").and_then(Json::as_array_mut) {
                items.push(json!("appended"));
            }
        }
        6 => {
            if let Some(rows) = map.get_mut("rows").and_then(Json::as_array_mut) {
                rows.push(json!({"cells": ["appended"]}));
            }
        }
        _ => {
            if next() % 2 == 0 {
                map.insert("fallback".into(), json!({"t": "Spinner"}));
            } else {
                map.remove("fallback");
            }
        }
    }
    new
}

/// Wraps `target` (keyed `"t"`) next to a 400-char filler so the §4.7 rule-4
/// size fallback (ops > 50 % of the encoded new node ⇒ replace) does not kick
/// in for the tiny op batches under test. The changed node's path is `["t"]`.
fn padded(target: &Json) -> Node {
    node(json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "pad", "text": "p".repeat(400)},
        target
    ]}))
}

/// The padded child JSON for a keyed `"t"` target.
fn padded_target(target: &Json) -> Json {
    let mut obj = target.clone();
    obj.as_object_mut()
        .expect("target is an object")
        .insert("key".into(), json!("t"));
    obj
}

// --------------------------------------------------------------- proptest

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 10_000,
        ..proptest::test_runner::Config::default()
    })]

    /// §4.7 invariant: `apply(old, diff(old, new)) == new` (10k cases, WP-U2).
    #[test]
    fn apply_inverts_diff(old in node_json(), new_seed in any::<u64>()) {
        let new = mutate(&old, new_seed);
        let old_node = serde_json::from_value::<Node>(old.clone());
        let new_node = serde_json::from_value::<Node>(new);
        prop_assume!(
            old_node.is_ok() && new_node.is_ok(),
            "generated nodes parse (generator emits valid shapes)"
        );
        roundtrip(&old_node.expect("old parses"), &new_node.expect("new parses"));
    }
}

// ----------------------------------------------------------- unit tests

#[test]
fn identical_nodes_diff_to_no_ops() {
    let tree = node(json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "x"},
        {"t": "List", "items": ["one", "two"]}
    ]}));
    let ops = roundtrip(&tree, &tree);
    assert_eq!(ops.len(), 0);
}

#[test]
fn scalar_prop_change_is_one_set() {
    let old = padded(&padded_target(
        &json!({"t": "Paragraph", "text": "x", "style": {"fg": "accent"}}),
    ));
    let new = padded(&padded_target(
        &json!({"t": "Paragraph", "text": "y", "style": {"fg": "accent"}}),
    ));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0],
        NodeOp::Set {
            path: vec!["t".to_owned()],
            props: serde_json::Map::from_iter([("text".to_owned(), json!("y"))]),
        }
    );
}

#[test]
fn removed_prop_unsets_through_apply() {
    let old = padded(&padded_target(
        &json!({"t": "Paragraph", "text": "x", "style": {"fg": "accent"}}),
    ));
    let new = padded(&padded_target(&json!({"t": "Paragraph", "text": "x"})));
    let ops = roundtrip(&old, &new);
    assert_eq!(
        ops,
        vec![NodeOp::Set {
            path: vec!["t".to_owned()],
            props: serde_json::Map::from_iter([("style".to_owned(), Json::Null)]),
        }]
    );
    // `null` unsets: applying the raw op must drop the prop.
    let applied = apply(&old, &ops).expect("applies");
    assert_eq!(applied, new);
}

#[test]
fn tag_or_key_change_replaces_at_the_position() {
    let old = padded(&padded_target(&json!({"t": "Paragraph", "text": "x"})));
    // tag change at ["t"]
    let new_tag = padded(&padded_target(&json!({"t": "Spinner"})));
    let ops = roundtrip(&old, &new_tag);
    assert_eq!(
        ops,
        vec![NodeOp::Replace {
            path: vec!["t".to_owned()],
            node: node(json!({"t": "Spinner", "key": "t"})),
        }]
    );
    // sibling key change: identity is the key, so this is a splice at the tail
    let new_key = node(json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "pad", "text": "p".repeat(400)},
        {"t": "Paragraph", "key": "c", "text": "x"}
    ]}));
    let ops = roundtrip(&old, &new_key);
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(&ops[0], NodeOp::Splice { index: 1, remove: 1, insert, .. }
        if matches!(insert.as_slice(), [PatchItem::Node(node)] if node.common().key == Some("c")))
    );
}

#[test]
fn keyed_child_change_recurses_to_a_set() {
    let old = padded(&padded_target(&json!({"t": "Paragraph", "text": "x"})));
    let new = padded(&padded_target(
        &json!({"t": "Paragraph", "text": "changed"}),
    ));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Set { path, .. } if path == &["t".to_owned()]));
}

#[test]
fn tail_append_is_append_for_items_rows_lines() {
    let old = padded(&padded_target(&json!({"t": "Log", "lines": ["a", "b"]})));
    let new = padded(&padded_target(
        &json!({"t": "Log", "lines": ["a", "b", "c"]}),
    ));
    let ops = roundtrip(&old, &new);
    assert_eq!(
        ops,
        vec![NodeOp::Append {
            path: vec!["t".to_owned()],
            field: PatchField::Lines,
            items: vec![PatchItem::Item(json!("c"))],
        }]
    );

    let old_list = padded(&padded_target(
        &json!({"t": "List", "items": [{"key": "k1", "content": "a"}]}),
    ));
    let new_list = padded(&padded_target(&json!({"t": "List", "items": [
        {"key": "k1", "content": "a"}, {"key": "k2", "content": "b"}
    ]})));
    let ops = roundtrip(&old_list, &new_list);
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        ops[0],
        NodeOp::Append {
            field: PatchField::Items,
            ..
        }
    ));

    let old_rows = padded(&padded_target(
        &json!({"t": "Table", "rows": [{"cells": ["a"]}], "widths": [10]}),
    ));
    let new_rows = padded(&padded_target(&json!({"t": "Table", "rows": [
        {"cells": ["a"]}, {"cells": ["b"]}
    ], "widths": [10]})));
    let ops = roundtrip(&old_rows, &new_rows);
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        ops[0],
        NodeOp::Append {
            field: PatchField::Rows,
            ..
        }
    ));
}

#[test]
fn changed_prefix_disables_the_append_fast_path() {
    // Not "only a tail append" (§4.7): the first line changed too ⇒ splice.
    let old = padded(&padded_target(&json!({"t": "Log", "lines": ["a", "b"]})));
    let new = padded(&padded_target(
        &json!({"t": "Log", "lines": ["a2", "b", "c"]}),
    ));
    let ops = roundtrip(&old, &new);
    assert!(
        ops.iter().all(|op| matches!(op, NodeOp::Splice { .. })),
        "{ops:?}"
    );
}

#[test]
fn reordered_keyed_members_splice_per_run() {
    // [a b] -> [b a]: the LIS keeps b; removing a and appending a is one run each.
    let old = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "1"},
        {"t": "Paragraph", "key": "b", "text": "2"}
    ]}));
    let new = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "b", "text": "2"},
        {"t": "Paragraph", "key": "a", "text": "1"}
    ]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 2, "one splice per maximal run");
    assert!(ops.iter().all(|op| matches!(
        op,
        NodeOp::Splice {
            field: PatchField::Children,
            ..
        }
    )));
}

#[test]
fn splice_keeps_lis_stable_members_in_place() {
    // [a b c] -> [a c d]: b removed, d appended, a and c untouched.
    let old = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "1"},
        {"t": "Paragraph", "key": "b", "text": "2"},
        {"t": "Paragraph", "key": "c", "text": "3"}
    ]}));
    let new = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "1"},
        {"t": "Paragraph", "key": "c", "text": "3"},
        {"t": "Paragraph", "key": "d", "text": "4"}
    ]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 2);
    assert!(
        matches!(ops[0], NodeOp::Splice { index: 1, remove: 1, ref insert, .. } if insert.is_empty())
    );
    assert!(matches!(
        ops[0],
        NodeOp::Splice {
            field: PatchField::Children,
            ..
        }
    ));

    // [a x b] -> [a y b]: one combined run keeps the surrounding keys.
    let old = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "1"},
        {"t": "Paragraph", "key": "xk", "text": "x"},
        {"t": "Paragraph", "key": "b", "text": "2"}
    ]}));
    let new = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "a", "text": "1"},
        {"t": "Paragraph", "key": "yk", "text": "y"},
        {"t": "Paragraph", "key": "b", "text": "2"}
    ]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1, "removal and insertion merge into one run");
    assert!(matches!(
        ops[0],
        NodeOp::Splice {
            index: 1,
            remove: 1,
            ..
        }
    ));
}

#[test]
fn stable_member_change_composes_with_a_splice() {
    // [k1 k2] -> [k1' k2 k3]: tail splice + keyed set for k1.
    let old = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "k1", "text": "old"},
        {"t": "Paragraph", "key": "k2", "text": "2"}
    ]}));
    let new = padded(&json!({"t": "Layout", "children": [
        {"t": "Paragraph", "key": "k1", "text": "new"},
        {"t": "Paragraph", "key": "k2", "text": "2"},
        {"t": "Paragraph", "key": "k3", "text": "3"}
    ]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 2);
    assert!(matches!(
        ops[0],
        NodeOp::Splice {
            index: 2,
            remove: 0,
            ..
        }
    ));
    assert!(
        matches!(&ops[1], NodeOp::Set { path, .. } if path == &["#1".to_owned(), "k1".to_owned()])
    );
}

#[test]
fn oversized_op_batch_falls_back_to_replace() {
    // §4.7 rule 4: ops > 50 % of the encoded new node ⇒ a single replace.
    let long = "x".repeat(4_096);
    let old = node(json!({"t": "Paragraph", "text": "y"}));
    let new = node(json!({"t": "Paragraph", "text": long}));
    let ops = roundtrip(&old, &new);
    assert_eq!(
        ops,
        vec![NodeOp::Replace {
            path: NodePath::new(),
            node: new.clone(),
        }]
    );
}

#[test]
fn apply_rejects_unresolvable_ops() {
    let tree =
        node(json!({"t": "Layout", "children": [{"t": "Paragraph", "key": "a", "text": "x"}]}));
    let cases = [
        NodeOp::Set {
            path: vec!["nope".to_owned()],
            props: serde_json::Map::new(),
        },
        NodeOp::Splice {
            path: NodePath::new(),
            field: PatchField::Children,
            index: 5,
            remove: 1,
            insert: vec![],
        },
        NodeOp::Item {
            path: NodePath::new(),
            field: PatchField::Children,
            key: "zz".to_owned(),
            value: PatchItem::Item(json!({"t": "Paragraph", "text": "n"})),
        },
        NodeOp::Append {
            path: NodePath::new(),
            field: PatchField::Lines,
            items: vec![PatchItem::Item(json!("x"))],
        },
    ];
    for op in &cases {
        assert!(
            apply(&tree, std::slice::from_ref(op)).is_err(),
            "{op:?} must not resolve"
        );
    }
    // wrong field for the kind
    let err = apply(
        &tree,
        &[NodeOp::Splice {
            path: vec!["a".to_owned()],
            field: PatchField::Items,
            index: 0,
            remove: 0,
            insert: vec![PatchItem::Item(json!("x"))],
        }],
    )
    .expect_err("a Paragraph has no items");
    assert!(matches!(err, PatchError::FieldMismatch { .. }));
}

#[test]
fn apply_resolves_the_fixed_path_segments() {
    let tree = padded(&padded_target(
        &json!({"t": "Block", "child": {"t": "Paragraph", "text": "in"}}),
    ));
    let new = padded(&padded_target(
        &json!({"t": "Block", "child": {"t": "Paragraph", "text": "out"}}),
    ));
    // `child` is an object prop (rule 2), not a keyed collection:
    let ops = diff(&tree, &new);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Set { path, props, .. }
        if path == &["t".to_owned()] && props.contains_key("child")));
    assert_eq!(apply(&tree, &ops).expect("applies"), new);
    // But foreign ops may address the fixed segments directly (§4.6):
    let manual = NodeOp::Set {
        path: vec!["t".to_owned(), "child".to_owned()],
        props: serde_json::Map::from_iter([("text".to_owned(), json!("out"))]),
    };
    assert_eq!(apply(&tree, &[manual]).expect("applies"), new);

    // "fallback" segment: same prop-diff shape, manual deep apply.
    let with_fallback = padded(&padded_target(&json!({"t": "Block",
        "child": {"t": "Paragraph", "text": "in"}, "fallback": {"t": "Spinner"}})));
    let with_empty = padded(&padded_target(&json!({"t": "Block",
        "child": {"t": "Paragraph", "text": "in"}, "fallback": {"t": "Empty"}})));
    let ops = diff(&with_fallback, &with_empty);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Set { path, props, .. }
        if path == &["t".to_owned()] && props.contains_key("fallback")));
    let manual = NodeOp::Replace {
        path: vec!["t".to_owned(), "fallback".to_owned()],
        node: node(json!({"t": "Empty"})),
    };
    assert_eq!(
        apply(&with_fallback, &[manual]).expect("applies"),
        with_empty
    );

    // "action" segment on Empty: manual deep apply.
    let empty = padded(&padded_target(
        &json!({"t": "Empty", "action": {"t": "Button", "label": "go", "action": "go"}}),
    ));
    let manual = NodeOp::Set {
        path: vec!["t".to_owned(), "action".to_owned()],
        props: serde_json::Map::from_iter([("label".to_owned(), json!("stop"))]),
    };
    assert_eq!(
        apply(&empty, &[manual]).expect("applies"),
        padded(&padded_target(
            &json!({"t": "Empty", "action": {"t": "Button", "label": "stop", "action": "go"}})
        ))
    );

    // pane key segment: keyed collection, so diff recurses naturally.
    let tabs = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}],
        "panes": {"t1": {"t": "Paragraph", "key": "t1", "text": "a"}}}),
    ));
    let new_tabs = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}],
        "panes": {"t1": {"t": "Paragraph", "key": "t1", "text": "b"}}}),
    ));
    let ops = diff(&tabs, &new_tabs);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Set { path, .. }
        if path == &["t".to_owned(), "t1".to_owned()]));
    assert_eq!(apply(&tabs, &ops).expect("applies"), new_tabs);
}

#[test]
fn panes_boundaries_round_trip() {
    // None -> Some(map): one splice inserting all keyed panes.
    let old = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}]}),
    ));
    let new = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}],
        "panes": {"t1": {"t": "Paragraph", "key": "t1", "text": "a"}, "t2": {"t": "Paragraph", "key": "t2", "text": "b"}}}),
    ));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        ops[0],
        NodeOp::Splice {
            field: PatchField::Panes,
            index: 0,
            remove: 0,
            ..
        }
    ));

    // Some(map) -> None: splice removing everything.
    let ops = roundtrip(&new, &old);
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(ops[0], NodeOp::Splice { field: PatchField::Panes, remove: 2, ref insert, .. }
            if insert.is_empty())
    );

    // Keyless pane inserts cannot be expressed ⇒ whole-node replace (§4.7).
    let keyed = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}],
        "panes": {"t1": {"t": "Paragraph", "key": "t1", "text": "a"}}}),
    ));
    let keyless = padded(&padded_target(
        &json!({"t": "Tabs", "titles": [{"key": "t1", "title": "T"}],
        "panes": {"t1": {"t": "Paragraph", "text": "a"}, "t2": {"t": "Paragraph", "text": "b"}}}),
    ));
    let ops = diff(&keyed, &keyless);
    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], NodeOp::Replace { .. }));
    assert_eq!(apply(&keyed, &ops).expect("applies"), keyless);
}

#[test]
fn item_op_replaces_a_keyed_row() {
    let old = padded(&json!({"t": "Table", "rows": [
        {"cells": ["a"], "key": "r1"}, {"cells": ["b"], "key": "r2"}
    ], "widths": [10]}));
    let new = padded(&json!({"t": "Table", "rows": [
        {"cells": ["a2"], "key": "r1"}, {"cells": ["b"], "key": "r2"}
    ], "widths": [10]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Item { key, field: PatchField::Rows, .. } if key == "r1"));
}

#[test]
fn positional_collections_diff_by_position() {
    // Sparkline data has positional identity only: same sequence ⇒ per-point
    // `item` ops keyed "#<index>", nulls included.
    let old = padded(&padded_target(
        &json!({"t": "Sparkline", "data": [1, null, 3]}),
    ));
    let new = padded(&padded_target(
        &json!({"t": "Sparkline", "data": [1, 2, 3]}),
    ));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], NodeOp::Item { field: PatchField::Data, key, .. } if key == "#1"));

    // Same-shape data diffs to nothing.
    let same = padded(&padded_target(
        &json!({"t": "Sparkline", "data": [1, null, 3]}),
    ));
    let ops = roundtrip(&old, &same);
    assert_eq!(ops.len(), 0);
}

#[test]
fn unknown_kinds_diff_like_known_ones() {
    let old = padded(&padded_target(
        &json!({"t": "Holgram", "style": {"fg": "accent"}}),
    ));
    let same_kind = padded(&padded_target(
        &json!({"t": "Holgram", "style": {"fg": "error"}}),
    ));
    let ops = roundtrip(&old, &same_kind);
    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], NodeOp::Set { .. }));

    let other_kind = padded(&padded_target(&json!({"t": "Wraith"})));
    let ops = roundtrip(&old, &other_kind);
    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], NodeOp::Replace { .. }));
}

#[test]
fn nested_tree_diffs_minimally() {
    let old = node(json!({"t": "Layout", "children": [
        {"t": "Layout", "key": "main", "children": [
            {"t": "Paragraph", "key": "head", "text": "h"},
            {"t": "List", "key": "body", "items": ["a", "b"]}
        ]},
        {"t": "Paragraph", "key": "foot", "text": "f"}
    ]}));
    let new = node(json!({"t": "Layout", "children": [
        {"t": "Layout", "key": "main", "children": [
            {"t": "Paragraph", "key": "head", "text": "h"},
            {"t": "List", "key": "body", "items": ["a", "b", "c"]}
        ]},
        {"t": "Paragraph", "key": "foot", "text": "f"}
    ]}));
    let ops = roundtrip(&old, &new);
    assert_eq!(ops.len(), 1);
    assert!(
        matches!(&ops[0], NodeOp::Append { path, field: PatchField::Items, .. }
        if path == &["main".to_owned(), "body".to_owned()])
    );
}

// ---------------------------------------------- layout vector coverage

/// The constraint universe the generator sweeps (ui-protocol §12.4).
const CANON: [Constraint; 11] = [
    Constraint::Length(20),
    Constraint::Percentage(50),
    Constraint::Percentage(0),
    Constraint::Percentage(100),
    Constraint::Ratio(1, 4),
    Constraint::Ratio(1, 0),
    Constraint::Min(20),
    Constraint::Max(20),
    Constraint::Fill(1),
    Constraint::Fill(0),
    Constraint::Fill(3),
];

#[derive(serde::Deserialize)]
struct VectorFile {
    vectors: Vec<LayoutVector>,
}

#[derive(serde::Deserialize)]
struct LayoutVector {
    name: String,
    direction: Direction,
    size: u16,
    constraints: Vec<Constraint>,
    flex: Flex,
    spacing: u16,
    margin: Option<Margin>,
    expected: Vec<[u16; 4]>,
}

#[test]
fn layout_vectors_cover_the_matrix() {
    let file: VectorFile =
        serde_json::from_str(include_str!("layout_vectors.json")).expect("vectors parse");
    let vectors = &file.vectors;
    assert!(
        vectors.len() >= 200,
        "need ≥ 200 vectors, got {}",
        vectors.len()
    );

    let mut names: Vec<&str> = vectors.iter().map(|v| v.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), vectors.len(), "vector names must be unique");

    for v in vectors {
        assert_eq!(
            v.constraints.len(),
            v.expected.len(),
            "vector {} must have one rect per constraint",
            v.name
        );
    }

    // Every constraint alone…
    for c in CANON {
        assert!(
            vectors
                .iter()
                .any(|v| v.constraints.len() == 1 && v.constraints[0] == c),
            "missing single-constraint vector for {c:?}"
        );
    }
    // …and paired with every other.
    for (i, first) in CANON.iter().enumerate() {
        for second in &CANON[i + 1..] {
            assert!(
                vectors.iter().any(|v| v.constraints.len() == 2
                    && ((v.constraints[0] == *first && v.constraints[1] == *second)
                        || (v.constraints[0] == *second && v.constraints[1] == *first))),
                "missing pair vector for {first:?} + {second:?}"
            );
        }
    }

    assert!(
        vectors
            .iter()
            .any(|v| v.constraints.contains(&Constraint::Fill(0)))
    );
    assert!(
        vectors.iter().any(|v| v
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::Ratio(_, 0)))),
        "missing Ratio(_, 0)"
    );
    assert!(
        vectors
            .iter()
            .any(|v| v.constraints.contains(&Constraint::Percentage(0)))
    );
    assert!(
        vectors
            .iter()
            .any(|v| v.constraints.contains(&Constraint::Percentage(100)))
    );

    layout_environment_coverage(vectors);
}

/// Every flex, spacing {0,1,3}, margins {0,1} + axis-split, sizes
/// {0,1,7,24,80,200} and both directions appear (§12.4).
fn layout_environment_coverage(vectors: &[LayoutVector]) {
    for flex in [
        Flex::Legacy,
        Flex::Start,
        Flex::End,
        Flex::Center,
        Flex::SpaceBetween,
        Flex::SpaceAround,
    ] {
        assert!(
            vectors.iter().any(|v| v.flex == flex),
            "missing flex {flex:?}"
        );
    }
    for spacing in [0u16, 1, 3] {
        assert!(
            vectors.iter().any(|v| v.spacing == spacing),
            "missing spacing {spacing}"
        );
    }
    assert!(
        vectors.iter().any(|v| v.margin.is_none()),
        "missing margin 0"
    );
    assert!(
        vectors.iter().any(|v| v.margin == Some(Margin::Uniform(1))),
        "missing margin 1"
    );
    assert!(
        vectors
            .iter()
            .any(|v| matches!(v.margin, Some(Margin::Axes { .. }))),
        "missing axis-split margin"
    );
    for size in [0u16, 1, 7, 24, 80, 200] {
        assert!(
            vectors.iter().any(|v| v.size == size),
            "missing size {size}"
        );
    }
    assert!(vectors.iter().any(|v| v.direction == Direction::Vertical));
    assert!(vectors.iter().any(|v| v.direction == Direction::Horizontal));
}
