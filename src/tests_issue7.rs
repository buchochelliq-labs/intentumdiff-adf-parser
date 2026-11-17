//! Exact regression pairs from buchochelliq-labs/intentumdiff-adf-parser#7.
//!
//! The issue's reproduction serialises one base pipeline with Python's
//! `json.dumps(base, indent=2)` and applies five independent single-field
//! edits. Each must reach the component tree as a changed payload leaf with an
//! exact source span; layout-only and identical inputs must not.
use super::*;
use std::collections::BTreeMap;

/// `json.dumps(base, indent=2)` from the issue, byte for byte.
const MULTI_LINE: &str = r#"{
  "name": "p",
  "properties": {
    "activities": [
      {
        "name": "Web",
        "type": "WebActivity",
        "typeProperties": {
          "url": "https://example.com/safe",
          "method": "POST"
        },
        "policy": {
          "retry": 0
        },
        "dependsOn": [
          {
            "activity": "Input",
            "dependencyConditions": [
              "Succeeded"
            ]
          }
        ]
      }
    ],
    "parameters": {
      "target": {
        "type": "string",
        "defaultValue": "safe"
      }
    },
    "variables": {
      "x": {
        "type": "String",
        "defaultValue": "safe"
      }
    }
  }
}"#;

/// `json.dumps(base)` from the issue (single line), byte for byte.
const SINGLE_LINE: &str = r#"{"name": "p", "properties": {"activities": [{"name": "Web", "type": "WebActivity", "typeProperties": {"url": "https://example.com/safe", "method": "POST"}, "policy": {"retry": 0}, "dependsOn": [{"activity": "Input", "dependencyConditions": ["Succeeded"]}]}], "parameters": {"target": {"type": "string", "defaultValue": "safe"}}, "variables": {"x": {"type": "String", "defaultValue": "safe"}}}}"#;

struct Edit {
    case: &'static str,
    /// Unique anchor before the edited token, in each layout.
    anchor_multi: &'static str,
    anchor_single: &'static str,
    old_token: &'static str,
    new_token: &'static str,
    kind: &'static str,
    old_label: &'static str,
    new_label: &'static str,
}

const EDITS: [Edit; 5] = [
    Edit {
        case: "adf_activity_url",
        anchor_multi: "\"url\": ",
        anchor_single: "\"url\": ",
        old_token: "\"https://example.com/safe\"",
        new_token: "\"https://example.com/danger\"",
        kind: "string",
        old_label: "https://example.com/safe",
        new_label: "https://example.com/danger",
    },
    Edit {
        case: "adf_policy_retry",
        anchor_multi: "\"retry\": ",
        anchor_single: "\"retry\": ",
        old_token: "0",
        new_token: "9",
        kind: "number",
        old_label: "0",
        new_label: "9",
    },
    Edit {
        case: "adf_dependency_condition",
        anchor_multi: "\"dependencyConditions\": [\n              ",
        anchor_single: "\"dependencyConditions\": [",
        old_token: "\"Succeeded\"",
        new_token: "\"Failed\"",
        kind: "string",
        old_label: "Succeeded",
        new_label: "Failed",
    },
    Edit {
        case: "adf_parameter_default",
        anchor_multi: "\"type\": \"string\",\n        \"defaultValue\": ",
        anchor_single: "\"type\": \"string\", \"defaultValue\": ",
        old_token: "\"safe\"",
        new_token: "\"danger\"",
        kind: "string",
        old_label: "safe",
        new_label: "danger",
    },
    Edit {
        case: "adf_variable_default",
        anchor_multi: "\"type\": \"String\",\n        \"defaultValue\": ",
        anchor_single: "\"type\": \"String\", \"defaultValue\": ",
        old_token: "\"safe\"",
        new_token: "\"danger\"",
        kind: "string",
        old_label: "safe",
        new_label: "danger",
    },
];

fn tree(source: &str) -> Value {
    let result: Value = serde_json::from_str(&AdfParser::process(
        source.to_owned(),
        "adf".to_owned(),
        "pipeline.json".to_owned(),
    ))
    .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    result
}

/// Every node keyed by semantic ID: (type, label, exact source text of its span).
fn nodes(source: &str) -> BTreeMap<String, (String, String, String)> {
    let output = tree(source);
    let lines: Vec<_> = source.split('\n').collect();
    let mut found = BTreeMap::new();
    let mut stack = vec![output];
    while let Some(mut node) = stack.pop() {
        let p = &node["position"];
        let (sl, sc, el, ec) = (
            p["start_line"].as_u64().unwrap() as usize,
            p["start_col"].as_u64().unwrap() as usize,
            p["end_line"].as_u64().unwrap() as usize,
            p["end_col"].as_u64().unwrap() as usize,
        );
        let text = if sl == el {
            lines[sl][sc..ec].to_owned()
        } else {
            let mut text = lines[sl][sc..].to_owned();
            for line in &lines[sl + 1..el] {
                text.push('\n');
                text.push_str(line);
            }
            text.push('\n');
            text.push_str(&lines[el][..ec]);
            text
        };
        let id = node["id"].as_str().unwrap().to_owned();
        let entry = (
            node["node_type"].as_str().unwrap().to_owned(),
            node["label"].as_str().unwrap().to_owned(),
            text,
        );
        assert!(found.insert(id, entry).is_none(), "duplicate semantic ID");
        if let Value::Array(children) = node["children"].take() {
            stack.extend(children);
        }
    }
    found
}

fn apply(base: &str, anchor: &str, old_token: &str, new_token: &str) -> String {
    let needle = format!("{anchor}{old_token}");
    assert_eq!(
        base.matches(&needle).count(),
        1,
        "anchor not unique: {needle}"
    );
    base.replacen(&needle, &format!("{anchor}{new_token}"), 1)
}

#[test]
fn issue_7_exact_pairs_change_one_payload_leaf_with_exact_spans() {
    for (layout, base) in [("multi", MULTI_LINE), ("single", SINGLE_LINE)] {
        for edit in &EDITS {
            let anchor = if layout == "multi" {
                edit.anchor_multi
            } else {
                edit.anchor_single
            };
            let changed = apply(base, anchor, edit.old_token, edit.new_token);
            let case = format!("{}[{layout}]", edit.case);
            assert_ne!(
                tree(base)["structural_hash"],
                tree(&changed)["structural_hash"],
                "{case}: edit certified as formatting"
            );
            let (old, new) = (nodes(base), nodes(&changed));
            assert_eq!(
                old.keys().collect::<Vec<_>>(),
                new.keys().collect::<Vec<_>>(),
                "{case}: single-field edit must not restructure the tree"
            );
            let leaves: Vec<_> = old
                .iter()
                .filter(|(id, (kind, label, _))| {
                    let (new_kind, new_label, _) = &new[*id];
                    kind != new_kind || label != new_label
                })
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(leaves.len(), 1, "{case}: changed labels {leaves:?}");
            let id = &leaves[0];
            assert_eq!(
                old[id],
                (
                    edit.kind.to_owned(),
                    edit.old_label.to_owned(),
                    edit.old_token.to_owned()
                ),
                "{case}: old payload witness"
            );
            assert_eq!(
                new[id],
                (
                    edit.kind.to_owned(),
                    edit.new_label.to_owned(),
                    edit.new_token.to_owned()
                ),
                "{case}: new payload witness"
            );
        }
    }
}

#[test]
fn issue_7_identity_and_layout_only_controls_are_not_semantic() {
    let hash = |source: &str| tree(source)["structural_hash"].clone();
    assert_eq!(hash(MULTI_LINE), hash(MULTI_LINE));
    // Same decoded document, different insignificant whitespace.
    assert_eq!(hash(MULTI_LINE), hash(SINGLE_LINE));
    assert_eq!(
        hash(MULTI_LINE),
        hash(&MULTI_LINE.replace("\n  ", "\n    "))
    );
    let labels = |source: &str| {
        nodes(source)
            .into_iter()
            .map(|(id, (kind, label, _))| (id, kind, label))
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(MULTI_LINE), labels(SINGLE_LINE));
}

#[test]
fn issue_7_quoted_delimiters_inside_payload_strings_are_compared_exactly() {
    let base = MULTI_LINE.replace("https://example.com/safe", r#"https://e.com/a\"//,}"#);
    let changed = base.replacen(r#"//,}""#, r#"//,]""#, 1);
    let (old, new) = (nodes(&base), nodes(&changed));
    let diff: Vec<_> = old
        .iter()
        // Container spans enclose the edited token; only labels may differ.
        .filter(|(id, (kind, label, _))| (&new[*id].0, &new[*id].1) != (kind, label))
        .map(|(id, entry)| (entry.clone(), new[id].clone()))
        .collect();
    assert_eq!(
        diff,
        vec![(
            (
                "string".to_owned(),
                r#"https://e.com/a"//,}"#.to_owned(),
                r#""https://e.com/a\"//,}""#.to_owned()
            ),
            (
                "string".to_owned(),
                r#"https://e.com/a"//,]"#.to_owned(),
                r#""https://e.com/a\"//,]""#.to_owned()
            ),
        )]
    );
}

#[test]
fn incomplete_source_returns_in_band_error_envelope() {
    // Core's `data_config.adf.incomplete` new side.
    for source in [
        "{\"name\": ",
        "{\"name\": \"p\", \"properties\": {\"activities\": [",
    ] {
        let output = AdfParser::process(
            source.to_owned(),
            "adf".to_owned(),
            "pipeline.json".to_owned(),
        );
        let envelope: Value = serde_json::from_str(&output).unwrap();
        let message = envelope["error"].as_str().unwrap_or_default();
        assert!(
            message.starts_with("JSON parse failed: EOF while parsing"),
            "{output}"
        );
        assert_eq!(envelope.as_object().unwrap().len(), 1, "{output}");
    }
}
