use super::*;

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

fn assert_payload_changes(old: &str, new: &str) {
    let old_tree = tree(old);
    let new_tree = tree(new);
    assert_ne!(
        old_tree["structural_hash"], new_tree["structural_hash"],
        "a declared configuration edit must reach the component's semantic tree"
    );
}

#[test]
fn parameter_defaults_participate_in_semantic_identity() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"parameters":{"folder":{"type":"string","defaultValue":"output"}}}}"#,
        r#"{"name":"P","properties":{"parameters":{"folder":{"type":"string","defaultValue":"archive"}}}}"#,
    );
}

#[test]
fn dependency_conditions_participate_in_semantic_identity() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","dependsOn":[{"activity":"B","dependencyConditions":["Succeeded"]}]}]}}"#,
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","dependsOn":[{"activity":"B","dependencyConditions":["Failed"]}]}]}}"#,
    );
}

#[test]
fn retry_policy_participates_in_semantic_identity() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","policy":{"retry":0}}]}}"#,
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","policy":{"retry":3}}]}}"#,
    );
}

#[test]
fn unicode_defaults_participate_in_semantic_identity() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"parameters":{"é/name":{"type":"string","defaultValue":"λ"}}}}"#,
        r#"{"name":"P","properties":{"parameters":{"é/name":{"type":"string","defaultValue":"μ"}}}}"#,
    );
}

#[test]
fn activity_endpoints_and_nested_extension_values_are_not_discarded() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","typeProperties":{"url":"https://example.test/a"}}],"extension":{"nested":{"value":true}}}}"#,
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"WebActivity","typeProperties":{"url":"https://example.test/b"}}],"extension":{"nested":{"value":true}}}}"#,
    );
    assert_payload_changes(
        r#"{"name":"P","properties":{"extension":{"nested":{"value":true}}}}"#,
        r#"{"name":"P","properties":{"extension":{"nested":{"value":false}}}}"#,
    );
}

#[test]
fn scalar_types_missing_values_and_sequence_order_remain_distinct() {
    let changes = [
        (
            r#"{"type":"string","defaultValue":false}"#,
            r#"{"type":"string","defaultValue":"false"}"#,
        ),
        (
            r#"{"type":"object","defaultValue":null}"#,
            r#"{"type":"object"}"#,
        ),
        (
            r#"{"type":"array","defaultValue":["a","b"]}"#,
            r#"{"type":"array","defaultValue":["b","a"]}"#,
        ),
        (
            r#"{"type":"int","defaultValue":18446744073709551616}"#,
            r#"{"type":"int","defaultValue":18446744073709551617}"#,
        ),
        (
            r#"{"type":"float","defaultValue":1.00000000000000001}"#,
            r#"{"type":"float","defaultValue":1.00000000000000002}"#,
        ),
    ];
    for (old, new) in changes {
        assert_payload_changes(
            &format!(r#"{{"name":"P","properties":{{"parameters":{{"p":{old}}}}}}}"#),
            &format!(r#"{{"name":"P","properties":{{"parameters":{{"p":{new}}}}}}}"#),
        );
    }
}

#[test]
fn object_order_spacing_and_equivalent_json_string_escapes_are_preserved() {
    let old = r#"{"name":"P","properties":{"parameters":{"p":{"type":"object","defaultValue":{"b":"λ","a":1}}}}}"#;
    let new = r#"{ "properties": { "parameters": { "p": { "defaultValue": {"a":1,"b":"\u03bb"}, "type":"object" } } }, "name":"P" }"#;
    assert_eq!(tree(old)["structural_hash"], tree(new)["structural_hash"]);
}

#[test]
fn added_payload_keeps_existing_parameter_type_identity() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"parameters":{"p":{"type":"string","defaultValue":"same"}}}}"#,
        r#"{"name":"P","properties":{"parameters":{"p":{"type":"secureString","defaultValue":"same"}}}}"#,
    );
}

#[test]
fn full_long_configuration_strings_retain_suffix_changes() {
    let source = |tail| {
        format!(
            r#"{{"name":"P","properties":{{"parameters":{{"p":{{"type":"string","defaultValue":"{}{tail}"}}}}}}}}"#,
            "a".repeat(4200),
        )
    };
    assert_payload_changes(&source('x'), &source('y'));
}

#[test]
fn payload_scalars_have_exact_utf8_source_spans_and_unique_ids() {
    let source = "{\n  \"name\": \"P\",\n  \"properties\": {\"parameters\": {\"é/name\": {\"type\": \"object\", \"defaultValue\": {\"é.name\": \"μ\", \"values\": [18446744073709551617, false, null, \"😀\"]}}}}\n}";
    let output = tree(source);
    let lines: Vec<_> = source.split('\n').collect();
    let mut stack = vec![&output];
    let mut ids = std::collections::HashSet::new();
    let mut witnesses = std::collections::HashMap::new();
    while let Some(node) = stack.pop() {
        assert!(
            ids.insert(node["id"].as_str().unwrap()),
            "duplicate ID: {node}"
        );
        let p = &node["position"];
        let start = (
            p["start_line"].as_u64().unwrap() as usize,
            p["start_col"].as_u64().unwrap() as usize,
        );
        let end = (
            p["end_line"].as_u64().unwrap() as usize,
            p["end_col"].as_u64().unwrap() as usize,
        );
        assert!(start <= end && end.0 < lines.len());
        assert!(lines[start.0].is_char_boundary(start.1));
        assert!(lines[end.0].is_char_boundary(end.1));
        if start.0 == end.0 {
            witnesses.insert(
                (
                    node["node_type"].as_str().unwrap(),
                    node["label"].as_str().unwrap(),
                ),
                &lines[start.0][start.1..end.1],
            );
        }
        assert!(node.get("facts").is_none() || node["facts"].is_null());
        stack.extend(node["children"].as_array().unwrap());
    }
    for (kind, label, token) in [
        ("string", "μ", "\"μ\""),
        ("string", "😀", "\"😀\""),
        ("number", "18446744073709551617", "18446744073709551617"),
        ("boolean", "false", "false"),
        ("null", "null", "null"),
    ] {
        assert_eq!(
            witnesses.get(&(kind, label)).copied(),
            Some(token),
            "missing exact payload witness {kind}/{label}"
        );
    }
}

#[test]
fn explicit_fields_cannot_collapse_into_absent_default_labels() {
    for (old, new) in [
        (
            r#"{"name":"","properties":{}}"#,
            r#"{"name":"pipeline","properties":{}}"#,
        ),
        (
            r#"{"properties":{}}"#,
            r#"{"name":"pipeline","properties":{}}"#,
        ),
        (
            r#"{"name":"P","properties":{}}"#,
            r#"{"name":"P","properties":{"parameters":{}}}"#,
        ),
        (
            r#"{"name":"P","properties":{"parameters":{"p":{}}}}"#,
            r#"{"name":"P","properties":{"parameters":{"p":{"type":"Any"}}}}"#,
        ),
    ] {
        assert_payload_changes(old, new);
    }
}

#[test]
fn ambiguous_duplicate_configuration_keys_return_an_explicit_error() {
    let source = r#"{"name":"P","properties":{"parameters":{"p":{"type":"object","defaultValue":{"x":1,"x":2}}}}}"#;
    let result: Value = serde_json::from_str(&AdfParser::process(
        source.to_owned(),
        "adf".to_owned(),
        "pipeline.json".to_owned(),
    ))
    .unwrap();
    assert!(result["error"].is_string());
}

#[test]
fn property_and_represented_field_anchors_point_to_exact_keys() {
    for key in ["left", "right", "é/name", "a\"b", "slash\\key", ""] {
        let key_token = serde_json::to_string(key).unwrap();
        let source = format!(
            r#"{{"name":"P","properties":{{"parameters":{{"p":{{"type":"object","defaultValue":{{{key_token}:1}}}}}}}}}}"#
        );
        let output = tree(&source);
        let mut stack = vec![&output];
        let mut found = false;
        while let Some(node) = stack.pop() {
            let kind = node["node_type"].as_str().unwrap();
            let label = node["label"].as_str().unwrap();
            if kind == "property" && label == key {
                let p = &node["position"];
                assert_eq!(p["start_line"], 0);
                assert_eq!(p["end_line"], 0);
                let start = p["start_col"].as_u64().unwrap() as usize;
                let end = p["end_col"].as_u64().unwrap() as usize;
                assert_eq!(&source[start..end], key_token);
                found = true;
            }
            if kind.ends_with("_field") {
                let p = &node["position"];
                let start = p["start_col"].as_u64().unwrap() as usize;
                let end = p["end_col"].as_u64().unwrap() as usize;
                assert_eq!(
                    serde_json::from_str::<String>(&source[start..end]).unwrap(),
                    label
                );
            }
            stack.extend(node["children"].as_array().unwrap());
        }
        assert!(found, "configuration key not represented");
    }
}

#[test]
fn unsupported_nonstring_annotations_are_explicit_in_band_errors() {
    let source = r#"{"name":"P","properties":{"annotations":[{"unsupported":1}]}}"#;
    let result: Value = serde_json::from_str(&AdfParser::process(
        source.to_owned(),
        "adf".to_owned(),
        "pipeline.json".to_owned(),
    ))
    .unwrap();
    assert_eq!(result["error"], "ADF annotations must be strings");
}

#[test]
fn duplicate_keys_at_every_domain_boundary_are_explicit_errors() {
    for source in [
        r#"{"name":"P","name":"Q","properties":{}}"#,
        r#"{"name":"P","properties":{"x":1,"x":2}}"#,
        r#"{"properties":{"parameters":{"p":{},"p":{}}}}"#,
        r#"{"properties":{"variables":{"v":{},"v":{}}}}"#,
        r#"{"properties":{"parameters":{"p":{"type":"string","type":"object"}}}}"#,
        r#"{"properties":{"activities":[{"name":"A","type":"Copy","type":"WebActivity"}]}}"#,
        r#"{"properties":{"activities":[{"inputs":[{"referenceName":"A","referenceName":"B"}]}]}}"#,
        r#"{"properties":{"activities":[{"dependsOn":[{"activity":"A","activity":"B"}]}]}}"#,
    ] {
        let result: Value = serde_json::from_str(&AdfParser::process(
            source.to_owned(),
            "adf".to_owned(),
            "pipeline.json".to_owned(),
        ))
        .unwrap();
        assert!(
            result["error"].is_string(),
            "ambiguous map accepted: {source}"
        );
    }
}

fn count_node(tree: &Value, kind: &str, label: &str) -> usize {
    usize::from(tree["node_type"] == kind && tree["label"] == label)
        + tree["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|child| count_node(child, kind, label))
            .sum::<usize>()
}

#[test]
fn mixed_reference_arrays_have_one_complete_projection() {
    let output = tree(
        r#"{"name":"P","properties":{"activities":[{"name":"task","type":"Copy","inputs":[{"referenceName":"A"},{}]}]}}"#,
    );
    assert_eq!(count_node(&output, "input_dataset", "A"), 0);
    assert_eq!(count_node(&output, "string", "A"), 1);
}

#[test]
fn mixed_activity_arrays_have_one_complete_projection() {
    let output =
        tree(r#"{"name":"P","properties":{"activities":[{"name":"A","type":"Copy"},null]}}"#);
    assert_eq!(count_node(&output, "activity", "A (Copy)"), 0);
    assert_eq!(count_node(&output, "string", "A"), 1);
}

#[test]
fn compound_activity_labels_cannot_hide_distinct_name_and_type() {
    assert_payload_changes(
        r#"{"name":"P","properties":{"activities":[{"name":"A (Copy)","type":""}]}}"#,
        r#"{"name":"P","properties":{"activities":[{"name":"A","type":"Copy"}]}}"#,
    );
}

#[test]
fn compound_declaration_labels_cannot_hide_distinct_key_and_type() {
    for collection in ["parameters", "variables"] {
        assert_payload_changes(
            &format!(r#"{{"name":"P","properties":{{"{collection}":{{"a : b":{{"type":"c"}}}}}}}}"#),
            &format!(r#"{{"name":"P","properties":{{"{collection}":{{"a":{{"type":"b : c"}}}}}}}}"#),
        );
    }
}
