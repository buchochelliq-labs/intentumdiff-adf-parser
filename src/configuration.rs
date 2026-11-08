//! ADF-specific selection and exact JSON source adaptation for shared SDK values.
use crate::json_spans::{JsonSpans, RawFields};
use intentumdiff_plugin_sdk::config::{
    ConfigEntry, ConfigKind, ConfigValue, ConfigurationProjector,
};
use intentumdiff_plugin_sdk::tree::{SemanticNode, SemanticNodeBuilder};
use serde_json::{value::RawValue, Map, Value};

fn positioned_value(raw: &RawValue, spans: &JsonSpans<'_>) -> Result<ConfigValue, &'static str> {
    let position = spans
        .position_raw(raw)
        .ok_or("Invalid configuration source span")?;
    let text = raw.get();
    let kind = match text.as_bytes().first().copied() {
        Some(b'{') => {
            let fields: RawFields<'_> =
                serde_json::from_str(text).map_err(|_| "Invalid configuration object")?;
            ConfigKind::Mapping(
                fields
                    .0
                    .into_iter()
                    .map(|(key, value)| {
                        Ok(ConfigEntry {
                            key_position: spans
                                .key_position_raw(&key, value)
                                .ok_or("Missing configuration key span")?,
                            value: positioned_value(value, spans)?,
                            key,
                        })
                    })
                    .collect::<Result<_, &'static str>>()?,
            )
        }
        Some(b'[') => {
            let values: Vec<&RawValue> =
                serde_json::from_str(text).map_err(|_| "Invalid configuration array")?;
            ConfigKind::Sequence(
                values
                    .into_iter()
                    .map(|value| positioned_value(value, spans))
                    .collect::<Result<_, _>>()?,
            )
        }
        Some(b'"') => ConfigKind::String(
            serde_json::from_str::<String>(text).map_err(|_| "Invalid configuration string")?,
        ),
        Some(b't') => ConfigKind::Boolean(true),
        Some(b'f') => ConfigKind::Boolean(false),
        Some(b'n') => ConfigKind::Null,
        // RawValue retains the exact number, including values that Value would round.
        Some(b'-' | b'0'..=b'9') => ConfigKind::Number(text.to_owned()),
        _ => return Err("Unsupported configuration value"),
    };
    Ok(ConfigValue { position, kind })
}

fn at_path<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut value = root;
    for part in path {
        value = match value {
            Value::Object(map) => map.get(*part)?,
            Value::Array(values) => values.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(value)
}

fn append_value(
    node: &mut SemanticNode,
    prefix: &str,
    name: &str,
    path: &[&str],
    spans: &mut JsonSpans<'_>,
    projector: &ConfigurationProjector<'_>,
) -> Result<(), &'static str> {
    let raw = spans
        .raw_at(path)
        .ok_or("Missing configuration source value")?;
    let value = positioned_value(raw, spans)?;
    let key_position = spans
        .key_position(path)
        .ok_or("Missing configuration key span")?;
    node.children.push(
        projector
            .configuration_node(prefix, name, &key_position, &value)
            .map_err(|_| "Configuration projection failed")?,
    );
    Ok(())
}

fn append_fields(
    node: &mut SemanticNode,
    scope: &str,
    path: &[&str],
    fields: &Map<String, Value>,
    represented: impl Fn(&str, &Value) -> bool,
    spans: &mut JsonSpans<'_>,
    projector: &ConfigurationProjector<'_>,
) -> Result<(), &'static str> {
    let names = spans
        .object_keys(path)
        .ok_or("Ambiguous ADF configuration mapping")?;
    for (index, name) in names.iter().enumerate() {
        let value = fields.get(name).ok_or("Missing ADF configuration field")?;
        let mut field_path = path.to_vec();
        field_path.push(name);
        if represented(name, value) {
            // The domain tree owns this value without an ambiguous compound
            // label; retain key/type presence to distinguish absent fields.
            let kind = match value {
                Value::String(_) => "string_field",
                Value::Array(_) => "array_field",
                Value::Object(_) => "object_field",
                _ => return Err("Unsupported represented ADF field"),
            };
            let position = spans
                .key_position(&field_path)
                .ok_or("Missing ADF field span")?;
            node.children.push(
                SemanticNodeBuilder::new(
                    format!("{}.config.{scope}.{index}.field", node.id),
                    kind,
                    name,
                    position.start_line,
                    position.start_col,
                    position.end_line,
                    position.end_col,
                    String::new(),
                )
                .build(),
            );
            continue;
        }
        append_value(
            node,
            &format!("{}.config.{scope}.{index}", node.id),
            name,
            &field_path,
            spans,
            projector,
        )?;
    }
    Ok(())
}

fn named_references(value: &Value, field: &str) -> bool {
    value.as_array().is_some_and(|values| {
        values.iter().all(|value| {
            value
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
    })
}

pub(crate) fn append_unrepresented(
    node: &mut SemanticNode,
    path: &[&str],
    root: &Value,
    spans: &mut JsonSpans<'_>,
    projector: &ConfigurationProjector<'_>,
) -> Result<(), &'static str> {
    let value = at_path(root, path).ok_or("Missing ADF configuration path")?;
    if let Some(fields) = value.as_object() {
        // A complete generic fallback replaces a partial domain-array projection.
        // Keeping both would report the same retained item twice.
        if node.node_type == "activity" {
            for (key, role, identity) in [
                ("inputs", "input_dataset", "referenceName"),
                ("outputs", "output_dataset", "referenceName"),
                ("dependsOn", "depends_on", "activity"),
            ] {
                if fields
                    .get(key)
                    .is_some_and(|value| !named_references(value, identity))
                {
                    node.children.retain(|child| child.node_type != role);
                }
            }
        } else if node.node_type == "pipeline"
            && fields
                .get("properties")
                .and_then(|p| p.get("activities"))
                .is_some_and(|value| {
                    !value
                        .as_array()
                        .is_some_and(|items| items.iter().all(Value::is_object))
                })
        {
            node.children.retain(|child| child.node_type != "activity");
        }
        match node.node_type.as_str() {
            "pipeline" => {
                append_fields(
                    node,
                    "root",
                    path,
                    fields,
                    |key, value| {
                        key == "name" && value.as_str().is_some_and(|name| !name.is_empty())
                            || key == "properties" && value.is_object()
                    },
                    spans,
                    projector,
                )?;
                if let Some(properties) = fields.get("properties").and_then(Value::as_object) {
                    append_fields(
                        node,
                        "properties",
                        &["properties"],
                        properties,
                        |key, value| match key {
                            "activities" => value
                                .as_array()
                                .is_some_and(|values| values.iter().all(Value::is_object)),
                            "parameters" | "variables" => value.is_object(),
                            "annotations" => value.is_array(),
                            _ => false,
                        },
                        spans,
                        projector,
                    )?;
                }
            }
            "activity" => append_fields(
                node,
                "activity",
                path,
                fields,
                |key, value| match key {
                    // Keep type independently: name + " (" + type + ")" is not
                    // an injective encoding of arbitrary accepted JSON strings.
                    "name" => value.is_string(),
                    "inputs" | "outputs" => named_references(value, "referenceName"),
                    "dependsOn" => named_references(value, "activity"),
                    _ => false,
                },
                spans,
                projector,
            )?,
            "parameter" | "variable" => append_fields(
                node,
                "declaration",
                path,
                fields,
                // The legacy "name : type" label likewise needs a separate
                // typed value to preserve each accepted declaration's identity.
                |_, _| false,
                spans,
                projector,
            )?,
            "input_dataset" | "output_dataset" => append_fields(
                node,
                "dataset",
                path,
                fields,
                |key, value| key == "referenceName" && value.is_string(),
                spans,
                projector,
            )?,
            "depends_on" => append_fields(
                node,
                "dependency",
                path,
                fields,
                |key, value| key == "activity" && value.is_string(),
                spans,
                projector,
            )?,
            "annotation" => return Err("ADF annotations must be strings"),
            _ => return Err("Unknown ADF configuration node"),
        }
    } else if matches!(node.node_type.as_str(), "parameter" | "variable") {
        append_value(
            node,
            &format!("{}.config.value", node.id),
            path.last().copied().ok_or("Missing ADF declaration key")?,
            path,
            spans,
            projector,
        )?;
    } else if node.node_type == "annotation" && !value.is_string() {
        return Err("ADF annotations must be strings");
    }

    // Use the SDK's one authoritative hash algorithm after adding configuration.
    let hashed = SemanticNodeBuilder::new(
        &node.id,
        &node.node_type,
        &node.label,
        node.position.start_line,
        node.position.start_col,
        node.position.end_line,
        node.position.end_col,
        String::new(),
    )
    .children(std::mem::take(&mut node.children))
    .build();
    node.structural_hash = hashed.structural_hash;
    node.children = hashed.children;
    Ok(())
}
