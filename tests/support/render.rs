use crate::{AnalysisError, FileFacts};
use serde_json::Value;

fn normalize(value: &mut Value, compact: bool) {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize(value, compact);
            }
        }
        Value::Object(fields) => {
            if fields.len() == 2 && fields.contains_key("start") && fields.contains_key("end") {
                *value = serde_json::json!([fields["start"], fields["end"]]);
                return;
            }
            let prefix = if fields.contains_key("resolution") {
                "r"
            } else if fields.contains_key("scope") {
                "d"
            } else if fields.contains_key("domain") {
                "s"
            } else if fields.contains_key("opening_span") {
                "e"
            } else if fields.contains_key("inline_attribute") {
                "style"
            } else if fields.contains_key("prelude") {
                "rule"
            } else if fields.contains_key("important") {
                "decl"
            } else {
                ""
            };
            let script = fields.contains_key("bundled");
            for (key, value) in fields.iter_mut() {
                normalize(value, compact);
                if !compact {
                    continue;
                }
                let label = match key.as_str() {
                    "id" | "parent" => prefix,
                    "scope" => "s",
                    "element" => "e",
                    "reference" => "r",
                    "style" => "style",
                    "rule" => "rule",
                    "declaration" => "decl",
                    _ => "",
                };
                if !label.is_empty() && value.is_u64() {
                    *value = Value::String(format!("{label}{}", value.as_u64().unwrap()));
                }
                if key == "reference" && value.is_object() {
                    *value = value["id"].clone();
                }
                if key == "Resolved" {
                    *value = value["id"].clone();
                }
                if matches!(key.as_str(), "references" | "injected" | "candidates")
                    && let Some(values) = value.as_array_mut()
                {
                    let label = if key == "references" { "r" } else { "d" };
                    for value in values {
                        if value.is_u64() {
                            *value = Value::String(format!("{label}{}", value.as_u64().unwrap()));
                        } else if script && key == "references" {
                            *value = value["id"].clone();
                        }
                    }
                }
            }
            // Dev dependencies may enable serde_json/preserve_order.
            // Keep the established snapshot order independent of feature unification.
            fields.sort_keys();
        }
        _ => {}
    }
}
pub fn outcome(result: &Result<FileFacts, AnalysisError>) -> Value {
    let mut value = serde_json::to_value(result).unwrap();
    normalize(&mut value, false);
    value
}
pub fn snapshot(result: &Result<FileFacts, AnalysisError>) -> String {
    let mut value = outcome(result);
    normalize(&mut value, true);
    let (status, fields) = value.as_object().unwrap().iter().next().unwrap();
    let entries = fields
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| {
            let rendered = if let Value::Array(values) = value {
                if values.is_empty() {
                    "[]".into()
                } else {
                    format!(
                        "[\n{}\n  ]",
                        values
                            .iter()
                            .map(|v| format!("    {}", serde_json::to_string(v).unwrap()))
                            .collect::<Vec<_>>()
                            .join(",\n")
                    )
                }
            } else {
                serde_json::to_string(value).unwrap()
            };
            format!("  {name:?}: {rendered}")
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{{status:?}: {{\n{entries}\n}}}}")
}
