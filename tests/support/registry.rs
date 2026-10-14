use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spec")
}
pub fn inventory() -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    fn walk(dir: &Path, prefix: &str, files: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
            if entry.file_type().unwrap().is_dir() {
                walk(&path, &format!("{name}/"), files);
            } else if path.extension().is_some_and(|e| e == "astro") {
                files.insert(name);
            }
        }
    }
    walk(&root(), "", &mut files);
    walk(&root().join("../fixtures"), "../fixtures/", &mut files);
    files
}
pub fn validate(cases: &Value, files: &BTreeSet<String>) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut registered = BTreeSet::new();
    for case in cases.as_array().ok_or("manifest must be an array")? {
        let id = case["id"].as_str().ok_or("missing id")?;
        let file = case["file"].as_str().ok_or("missing file")?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("invalid id: {id}"));
        }
        if !ids.insert(id) {
            return Err(format!("duplicate id: {id}"));
        }
        if !registered.insert(file.to_owned()) {
            return Err(format!("duplicate file: {file}"));
        }
        if !files.contains(file) {
            return Err(format!("missing or invalid fixture: {file}"));
        }
        if !matches!(
            case["snapshot_group"].as_str(),
            Some("template" | "script" | "style")
        ) {
            return Err(format!("invalid snapshot group: {id}"));
        }
        if !case["atlas"].is_object() || !case["compiler"].is_object() {
            return Err(format!("missing expectations: {id}"));
        }
    }
    if &registered != files {
        return Err(format!(
            "unregistered fixtures: {:?}",
            files.difference(&registered).collect::<Vec<_>>()
        ));
    }
    Ok(())
}
pub fn cases() -> Vec<Value> {
    let value =
        serde_json::from_str(&fs::read_to_string(root().join("manifest.json")).unwrap()).unwrap();
    validate(&value, &inventory()).unwrap();
    value.as_array().unwrap().clone()
}
pub fn source(case: &Value) -> String {
    fs::read_to_string(root().join(case["file"].as_str().unwrap())).unwrap()
}
