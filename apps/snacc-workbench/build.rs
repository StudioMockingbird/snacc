use serde_json::{Map, Value};
use std::{collections::HashSet, env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=page.html");
    println!("cargo:rerun-if-changed=snippets.json");
    println!("cargo:rerun-if-changed=../../examples");

    let catalog_path = PathBuf::from("snippets.json");
    let raw = fs::read_to_string(&catalog_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", catalog_path.display()));
    let entries = serde_json::from_str::<Value>(&raw)
        .unwrap_or_else(|error| panic!("invalid snippets.json: {error}"));
    let entries = entries
        .as_array()
        .unwrap_or_else(|| panic!("snippets.json must contain an array"));
    let examples = PathBuf::from("../../examples");
    let mut ids = HashSet::new();
    let mut embedded = Vec::new();

    for entry in entries {
        let object = entry
            .as_object()
            .unwrap_or_else(|| panic!("each snippet entry must be an object"));
        let id = required_string(object, "id");
        let name = required_string(object, "name");
        let description = required_string(object, "description");
        let case_name = required_string(object, "case");
        if id.is_empty() || name.is_empty() || description.is_empty() || case_name.is_empty() {
            panic!("snippet fields may not be empty");
        }
        if !ids.insert(id.to_string()) {
            panic!("duplicate snippet id '{id}'");
        }
        if case_name.contains('/') || case_name.contains('\\') || case_name.contains("..") {
            panic!("snippet case '{case_name}' must be a basename");
        }
        let source_path = examples.join(format!("{case_name}.nrs"));
        let expected_path = examples.join(format!("{case_name}.stdout"));
        let source = fs::read_to_string(&source_path).unwrap_or_else(|error| {
            panic!(
                "snippet '{id}' source '{}' is missing or unreadable: {error}",
                source_path.display()
            )
        });
        if !expected_path.is_file() {
            panic!(
                "snippet '{id}' expected output '{}' is missing",
                expected_path.display()
            );
        }
        let mut embedded_entry = Map::new();
        embedded_entry.insert("id".into(), Value::String(id.into()));
        embedded_entry.insert("name".into(), Value::String(name.into()));
        embedded_entry.insert("description".into(), Value::String(description.into()));
        embedded_entry.insert("source".into(), Value::String(source));
        embedded.push(Value::Object(embedded_entry));
    }

    let output = serde_json::to_vec(&embedded).expect("failed to encode embedded snippets");
    let output_path =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set")).join("snippets.json");
    fs::write(&output_path, output)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output_path.display()));
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> &'a str {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("snippet field '{key}' must be a string"))
}
