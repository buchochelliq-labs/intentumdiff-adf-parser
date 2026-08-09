//! Azure Data Factory pipeline parser plugin — full-parse mode.
//!
//! Handles Azure Data Factory pipeline JSON definition files.
//! Detects ADF files by content heuristic: JSON containing "Microsoft.DataFactory".
//!
//! No tree-sitter grammar exists for ADF pipelines; this plugin parses the JSON
//! structure directly using serde_json, producing domain-specific semantic nodes:
//!
//!   pipeline   — root node (label = pipeline name from `.name`)
//!   activity   — each entry in `.properties.activities` (label = activity name + type)
//!   parameter  — each entry in `.properties.parameters` (label = param name : type)
//!   variable   — each entry in `.properties.variables`  (label = var name : type)
//!   annotation — entries in `.properties.annotations`   (label = annotation value)

use intentumdiff_plugin_sdk::tree::{SemanticNode, SemanticNodeBuilder};
use serde_json::Value;

wit_bindgen::generate!({
    path: "wit/plugin.wit",
    world: "parser-plugin",
});

use crate::exports::intentdiff::plugin::parser::ExamplePair;
use crate::exports::intentdiff::plugin::parser::Guest;
use crate::exports::intentdiff::plugin::parser::LanguageInfoRecord;
use crate::exports::intentdiff::plugin::parser::ParserMode;

const PLUGIN_METADATA: &str = include_str!("../plugin_metadata.info");

fn language_info_for(ids: Vec<String>) -> Vec<LanguageInfoRecord> {
    let metadata = intentumdiff_plugin_sdk::metadata::parse_plugin_metadata(PLUGIN_METADATA);
    ids.into_iter()
        .map(|language_id| {
            let info = metadata.language_or_default(&language_id);
            LanguageInfoRecord {
                language_id: info.language_id,
                language_name: info.language_name,
                language_short_name: info.language_short_name,
                monaco_language: info.monaco_language,
                default_filename: info.default_filename,
                language_file_extensions: info.language_file_extensions,
                author: metadata.author().to_string(),
                plugin_version: metadata.plugin_version().to_string(),
                last_updated: metadata.last_updated().to_string(),
            }
        })
        .collect()
}
struct AdfParser;

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

fn is_adf(content: &str) -> bool {
    content.contains("Microsoft.DataFactory")
        || (content.contains("\"activities\"")
            && content.contains("\"typeProperties\"")
            && content.contains("\"type\""))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn str_field<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    map.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

fn leaf(id: &str, node_type: &str, label: &str) -> SemanticNode {
    SemanticNodeBuilder::new(id, node_type, label, 0, 0, 0, 0, String::new()).build()
}

fn parent_node(
    id: &str,
    node_type: &str,
    label: &str,
    children: Vec<SemanticNode>,
) -> SemanticNode {
    SemanticNodeBuilder::new(id, node_type, label, 0, 0, 0, 0, String::new())
        .children(children)
        .build()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_activity(id: &str, act: &Value) -> Option<SemanticNode> {
    let map = act.as_object()?;
    let name = str_field(map, "name");
    let kind = str_field(map, "type");
    let label = if kind.is_empty() {
        name.to_string()
    } else {
        format!("{} ({})", name, kind)
    };
    let mut children = Vec::new();

    // Inputs
    if let Some(Value::Array(inputs)) = map.get("inputs") {
        for (i, inp) in inputs.iter().enumerate() {
            if let Some(m) = inp.as_object() {
                let ref_name = str_field(m, "referenceName");
                if !ref_name.is_empty() {
                    children.push(leaf(
                        &format!("{}.input.{}", id, i),
                        "input_dataset",
                        ref_name,
                    ));
                }
            }
        }
    }
    // Outputs
    if let Some(Value::Array(outputs)) = map.get("outputs") {
        for (i, out) in outputs.iter().enumerate() {
            if let Some(m) = out.as_object() {
                let ref_name = str_field(m, "referenceName");
                if !ref_name.is_empty() {
                    children.push(leaf(
                        &format!("{}.output.{}", id, i),
                        "output_dataset",
                        ref_name,
                    ));
                }
            }
        }
    }
    // Dependencies
    if let Some(Value::Array(deps)) = map.get("dependsOn") {
        for (i, dep) in deps.iter().enumerate() {
            if let Some(m) = dep.as_object() {
                let dep_act = str_field(m, "activity");
                if !dep_act.is_empty() {
                    children.push(leaf(&format!("{}.dep.{}", id, i), "depends_on", dep_act));
                }
            }
        }
    }

    Some(parent_node(id, "activity", &label, children))
}

fn parse_parameter(id: &str, name: &str, def: &Value) -> SemanticNode {
    let type_str = def
        .as_object()
        .and_then(|m| m.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("Any");
    let label = format!("{} : {}", name, type_str);
    leaf(id, "parameter", &label)
}

fn parse_variable(id: &str, name: &str, def: &Value) -> SemanticNode {
    let type_str = def
        .as_object()
        .and_then(|m| m.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("Any");
    let label = format!("{} : {}", name, type_str);
    leaf(id, "variable", &label)
}

fn parse_adf(content: &str) -> String {
    let val: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => return format!(r#"{{"error":"JSON parse failed: {}"}}"#, e),
    };
    let root_map = match val.as_object() {
        Some(m) => m,
        None => return r#"{"error":"Not a JSON object"}"#.to_string(),
    };

    let pipeline_name = str_field(root_map, "name");
    let mut children: Vec<SemanticNode> = Vec::new();

    let props = root_map.get("properties").and_then(|v| v.as_object());

    // Activities
    if let Some(Value::Array(activities)) = props.and_then(|p| p.get("activities")) {
        for (i, act) in activities.iter().enumerate() {
            if let Some(node) = parse_activity(&format!("0.act.{}", i), act) {
                children.push(node);
            }
        }
    }

    // Parameters
    if let Some(Value::Object(params)) = props.and_then(|p| p.get("parameters")) {
        for (i, (name, def)) in params.iter().enumerate() {
            children.push(parse_parameter(&format!("0.param.{}", i), name, def));
        }
    }

    // Variables
    if let Some(Value::Object(vars)) = props.and_then(|p| p.get("variables")) {
        for (i, (name, def)) in vars.iter().enumerate() {
            children.push(parse_variable(&format!("0.var.{}", i), name, def));
        }
    }

    // Annotations
    if let Some(Value::Array(annotations)) = props.and_then(|p| p.get("annotations")) {
        for (i, ann) in annotations.iter().enumerate() {
            let label = match ann {
                Value::String(s) => s.clone(),
                _ => ann.to_string(),
            };
            children.push(leaf(&format!("0.ann.{}", i), "annotation", &label));
        }
    }

    let root = parent_node(
        "0",
        "pipeline",
        if pipeline_name.is_empty() {
            "pipeline"
        } else {
            pipeline_name
        },
        children,
    );

    match serde_json::to_string(&root) {
        Ok(s) => s,
        Err(e) => format!(r#"{{"error":"Serialisation error: {}"}}"#, e),
    }
}

impl Guest for AdfParser {
    fn get_parser_mode() -> ParserMode {
        ParserMode::FullParse
    }
    fn grammar_id() -> String {
        "adf".to_string()
    }
    fn detect_language(filename: String, content: String) -> String {
        let lower = filename.to_lowercase();
        if lower.ends_with(".json") && is_adf(&content) {
            return "adf".to_string();
        }
        String::new()
    }
    fn preprocess_source(source: String) -> String {
        source
    }
    fn process(input: String, _language: String, _filename: String) -> String {
        parse_adf(&input)
    }
    fn trivia_node_types() -> Vec<String> {
        vec![]
    }
    fn language_ids() -> Vec<String> {
        vec!["adf".to_string()]
    }
    fn language_info() -> Vec<LanguageInfoRecord> {
        language_info_for(Self::language_ids())
    }
    fn priority() -> i32 {
        0
    }

    fn example(_language: String) -> ExamplePair {
        ExamplePair {
            old: "{\n  \"name\": \"CopyPipeline\",\n  \"properties\": {\n    \"activities\": [\n      {\n        \"name\": \"CopyData\",\n        \"type\": \"Copy\",\n        \"inputs\":  [{\"referenceName\": \"Source\", \"type\": \"DatasetReference\"}],\n        \"outputs\": [{\"referenceName\": \"Sink\",   \"type\": \"DatasetReference\"}],\n        \"typeProperties\": {\"source\": {\"type\": \"BlobSource\"}, \"sink\": {\"type\": \"BlobSink\"}}\n      }\n    ]\n  }\n}\n".to_string(),
            new: "{\n  \"name\": \"CopyPipeline\",\n  \"properties\": {\n    \"parameters\": {\n      \"targetFolder\": {\"type\": \"string\", \"defaultValue\": \"output\"}\n    },\n    \"activities\": [\n      {\n        \"name\": \"CopyData\",\n        \"type\": \"Copy\",\n        \"inputs\":  [{\"referenceName\": \"Source\", \"type\": \"DatasetReference\"}],\n        \"outputs\": [{\"referenceName\": \"Sink\",   \"type\": \"DatasetReference\"}],\n        \"typeProperties\": {\"source\": {\"type\": \"BlobSource\"}, \"sink\": {\"type\": \"BlobSink\"}}\n      },\n      {\n        \"name\": \"LogSuccess\",\n        \"type\": \"WebActivity\",\n        \"dependsOn\": [{\"activity\": \"CopyData\", \"dependencyConditions\": [\"Succeeded\"]}],\n        \"typeProperties\": {\"url\": \"https://example.com/log\", \"method\": \"POST\"}\n      }\n    ]\n  }\n}\n".to_string(),
        }
    }
}
export!(AdfParser);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::intentdiff::plugin::parser::Guest;
    use intentumdiff_plugin_sdk::testing as t;

    #[test]
    fn grammar_id_nonempty() {
        assert!(!AdfParser::grammar_id().is_empty());
    }

    #[test]
    fn language_ids_contain_grammar_id() {
        let gid = AdfParser::grammar_id();
        let ids = AdfParser::language_ids();
        assert!(
            ids.contains(&gid),
            "language_ids {:?} must contain {:?}",
            ids,
            gid
        );
    }

    #[test]
    fn detect_language_known_ext() {
        // ADF detection requires JSON content with a DataFactory signature.
        let content = r#"{"$schema":"Microsoft.DataFactory"}"#;
        let r = AdfParser::detect_language("pipeline.json".to_string(), content.to_string());
        assert_eq!(r.as_str(), "adf");
    }

    #[test]
    fn detect_language_unknown_ext() {
        let r =
            AdfParser::detect_language("test.xyz_notareal_ext_9z8y".to_string(), "".to_string());
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn process_impl_empty_returns_valid_json() {
        let out = parse_adf("");
        t::assert_valid_json(&out, "process(empty)");
    }

    #[test]
    fn process_impl_whitespace_returns_valid_json() {
        let out = parse_adf("   \n  ");
        t::assert_valid_json(&out, "process(whitespace)");
    }
}
