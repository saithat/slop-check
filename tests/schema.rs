//! Real reports must match `docs/report.schema.json`, and the schema must name every
//! code the tool can emit.
//!
//! The validator implements only the keywords the schema uses and rejects any other,
//! so the schema cannot rely on a constraint this test does not enforce.

use serde_json::{Map, Value, json};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn schema() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/report.schema.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

struct Validator<'a> {
    root: &'a Value,
    errors: Vec<String>,
}

impl<'a> Validator<'a> {
    fn validate(root: &'a Value, value: &Value) -> Vec<String> {
        let mut validator = Self {
            root,
            errors: vec![],
        };
        validator.check(value, root, "$");
        validator.errors
    }

    fn resolve(&self, reference: &Value) -> &'a Value {
        let reference = reference.as_str().unwrap();
        let name = reference
            .strip_prefix("#/$defs/")
            .unwrap_or_else(|| panic!("unsupported $ref {reference}"));
        &self.root["$defs"][name]
    }

    /// Applies each keyword of `schema`, descending into referenced and child schemas.
    fn check(&mut self, value: &Value, schema: &Value, at: &str) {
        let keywords = schema.as_object().expect("schemas are objects");
        for (keyword, argument) in keywords {
            match keyword.as_str() {
                "$schema" | "$id" | "$defs" | "title" | "description" => {}
                "$ref" => self.check(value, self.resolve(argument), at),
                "items" => self.check_items(value, argument, at),
                "properties" => self.check_properties(value, argument, at),
                _ => self.errors.extend(
                    violation(keyword, argument, value, keywords)
                        .map(|problem| format!("{at}: {problem}")),
                ),
            }
        }
    }

    fn check_items(&mut self, value: &Value, schema: &Value, at: &str) {
        for (index, item) in value.as_array().into_iter().flatten().enumerate() {
            self.check(item, schema, &format!("{at}[{index}]"));
        }
    }

    fn check_properties(&mut self, value: &Value, properties: &Value, at: &str) {
        for (key, schema) in properties.as_object().unwrap() {
            if let Some(field) = value.get(key) {
                self.check(field, schema, &format!("{at}.{key}"));
            }
        }
    }
}

/// How `value` breaks a keyword that constrains it directly, if it does.
fn violation(
    keyword: &str,
    argument: &Value,
    value: &Value,
    keywords: &Map<String, Value>,
) -> Option<String> {
    match keyword {
        "type" => (!types(argument).iter().any(|name| has_type(value, name)))
            .then(|| format!("{value} is not of type {argument}")),
        "const" => (value != argument).then(|| format!("{value} is not {argument}")),
        "enum" => (!argument.as_array().unwrap().contains(value))
            .then(|| format!("{value} is not in {argument}")),
        "minimum" => value
            .as_f64()
            .filter(|number| *number < argument.as_f64().unwrap())
            .map(|_| format!("{value} is below {argument}")),
        "minItems" => value
            .as_array()
            .filter(|items| (items.len() as u64) < argument.as_u64().unwrap())
            .map(|_| format!("fewer than {argument} items")),
        "required" => {
            let object = value.as_object()?;
            let missing: Vec<_> = argument
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .filter(|key| !object.contains_key(*key))
                .collect();
            (!missing.is_empty()).then(|| format!("missing {missing:?}"))
        }
        "additionalProperties" => {
            assert_eq!(argument, &json!(false), "only `false` is supported");
            let known = keywords.get("properties").and_then(Value::as_object);
            let unexpected: Vec<_> = value
                .as_object()?
                .keys()
                .filter(|key| !known.is_some_and(|known| known.contains_key(*key)))
                .collect();
            (!unexpected.is_empty()).then(|| format!("unexpected keys {unexpected:?}"))
        }
        other => Some(format!("unsupported keyword {other}")),
    }
}

fn types(argument: &Value) -> Vec<&str> {
    match argument {
        Value::String(name) => vec![name],
        Value::Array(names) => names.iter().map(|name| name.as_str().unwrap()).collect(),
        other => panic!("unsupported type {other}"),
    }
}

fn has_type(value: &Value, name: &str) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        other => panic!("unsupported type {other}"),
    }
}

/// A report that exercises every section: each observation kind, several rules, every
/// language, resolved edges, a cycle, and diagnostics with and without lines.
fn fixture_report() -> Value {
    let root = TempDir::new().unwrap();
    let write = |path: &str, source: &[u8]| {
        let target = root.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, source).unwrap();
    };
    let mut branchy = String::from("def branch(values):\n");
    for index in 0..15 {
        branchy.push_str(&format!("    if values[{index}]: return {index}\n"));
    }
    branchy.push_str("    return -1\n");
    write("complex.py", branchy.as_bytes());
    write(
        "rules.py",
        b"def route(a, b, c, d, e, f):\n    a = a\n    try:\n        b()\n    except Exception:\n        pass\n    if a == 'x':\n        return 1\n    elif a == 'y':\n        return 2\n    elif a == 'z':\n        return 3\n",
    );
    write(
        "rules.rs",
        b"fn pair() -> (u8, u8, bool) { (1, 2, true) }\nfn get(v: ((u8, u8), u8)) -> u8 { v.0.1 }\nfn name(k: u8) -> u8 { match k { 0 => 1, _ => unreachable!() } }\nfn early() { return; drop(1); }\n",
    );
    for (path, name) in [("a.py", "alpha"), ("b.py", "beta")] {
        let mut source = format!("def {name}(x):\n");
        for index in 0..10 {
            source.push_str(&format!("    x{index} = x + {index}\n"));
        }
        source.push_str("    return x9\n");
        write(path, source.as_bytes());
    }
    write("cycle/x.py", b"from . import y\n");
    write("cycle/y.py", b"from . import x\n");
    write(
        "app/main.ts",
        b"import { f } from './lib.js';\nexport const g = f;\n",
    );
    write("app/lib.ts", b"export const f = 1;\n");
    write("app/view.tsx", b"export const V = () => <b/>;\n");
    write("app/util.js", b"module.exports = {};\n");
    write("tool.go", b"package p\nfunc F(x int) int { return x }\n");
    write("bad.py", b"def broken(\n");
    write("latin1.py", b"x = '\xe9'\n");

    let output = Command::new(env!("CARGO_BIN_EXE_slop-check"))
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "the fixture has diagnostics");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn reports_match_the_schema() {
    let schema = schema();
    let report = fixture_report();

    let errors = Validator::validate(&schema, &report);
    assert!(errors.is_empty(), "{errors:#?}");

    // The fixture must exercise what the schema describes.
    let kinds: Vec<_> = report["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|observation| observation["kind"].as_str().unwrap())
        .collect();
    for kind in schema["$defs"]["observation_kind"]["enum"]
        .as_array()
        .unwrap()
    {
        assert!(
            kinds.contains(&kind.as_str().unwrap()),
            "no {kind} observation"
        );
    }
    for language in schema["$defs"]["language"]["enum"].as_array().unwrap() {
        let language = language.as_str().unwrap();
        assert!(
            report["by_language"].get(language).is_some(),
            "no {language} file"
        );
    }
    assert!(
        !report["dependencies"]["resolved_edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let diagnostic_lines: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| &diagnostic["line"])
        .collect();
    assert_eq!(diagnostic_lines, [&json!(1), &json!(1)]);
}

#[test]
fn the_schema_lists_every_code_the_report_indexes() {
    let schema = schema();
    let report = fixture_report();
    let index = report["observation_codes"].as_array().unwrap();
    let codes: Vec<_> = index.iter().map(|entry| &entry["code"]).collect();
    let listed: Vec<_> = schema["$defs"]["observation_code"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(codes, listed);

    let rules: Vec<_> = index
        .iter()
        .filter(|entry| entry["kind"] == "structural_finding")
        .map(|entry| &entry["code"])
        .collect();
    let listed: Vec<_> = schema["$defs"]["rule"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(rules, listed);

    let languages: Vec<_> = schema["properties"]["by_language"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    let mut listed: Vec<_> = schema["$defs"]["language"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|language| language.as_str().unwrap())
        .collect();
    listed.sort_unstable();
    assert_eq!(languages, listed);
}

#[test]
fn the_validator_rejects_reports_that_break_the_schema() {
    let schema = schema();
    let report = fixture_report();
    let broken = |edit: &dyn Fn(&mut Value)| {
        let mut copy = report.clone();
        edit(&mut copy);
        Validator::validate(&schema, &copy)
    };
    assert!(!broken(&|r| r["extra"] = json!(1)).is_empty());
    assert!(
        !broken(&|r| {
            r.as_object_mut().unwrap().remove("files");
        })
        .is_empty()
    );
    assert!(!broken(&|r| r["observations"][0]["code"] = json!("unknown")).is_empty());
    assert!(!broken(&|r| r["files"][0]["functions"][0]["start_line"] = json!(0)).is_empty());
    assert!(!broken(&|r| r["observations"][0]["symbol"] = json!(3)).is_empty());
}
