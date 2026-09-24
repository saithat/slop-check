//! Version identifiers recorded in every report.

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Changes whenever a metric or rule definition changes.
pub const METRIC_VERSION: &str = "metrics/5";
/// Changes whenever the JSON report's shape changes.
pub const SCHEMA_VERSION: u32 = 6;

/// Every source file of the analyzer, plus the lockfile. A test keeps this list complete.
const SOURCES: &[(&str, &str)] = &[
    ("Cargo.lock", include_str!("../Cargo.lock")),
    (
        "src/analyze/complexity.rs",
        include_str!("analyze/complexity.rs"),
    ),
    (
        "src/analyze/functions.rs",
        include_str!("analyze/functions.rs"),
    ),
    ("src/analyze/imports.rs", include_str!("analyze/imports.rs")),
    ("src/analyze/mod.rs", include_str!("analyze/mod.rs")),
    ("src/analyze/python.rs", include_str!("analyze/python.rs")),
    ("src/analyze/rules.rs", include_str!("analyze/rules.rs")),
    ("src/analyze/tests.rs", include_str!("analyze/tests.rs")),
    (
        "src/analyze/traversal.rs",
        include_str!("analyze/traversal.rs"),
    ),
    ("src/dependencies.rs", include_str!("dependencies.rs")),
    ("src/language.rs", include_str!("language.rs")),
    ("src/lib.rs", include_str!("lib.rs")),
    ("src/lines.rs", include_str!("lines.rs")),
    ("src/main.rs", include_str!("main.rs")),
    ("src/model.rs", include_str!("model.rs")),
    ("src/observation.rs", include_str!("observation.rs")),
    ("src/path.rs", include_str!("path.rs")),
    ("src/report/clones.rs", include_str!("report/clones.rs")),
    ("src/report/mod.rs", include_str!("report/mod.rs")),
    ("src/report/summary.rs", include_str!("report/summary.rs")),
    ("src/report/tests.rs", include_str!("report/tests.rs")),
    ("src/scan.rs", include_str!("scan.rs")),
    ("src/syntax.rs", include_str!("syntax.rs")),
    ("src/testing.rs", include_str!("testing.rs")),
    ("src/version.rs", include_str!("version.rs")),
];

/// Hashes the metric version, the analyzer's sources, and `Cargo.lock`. This tells
/// source and dependency revisions apart, while the package version stays the concise
/// human-facing identifier.
pub fn analyzer_fingerprint() -> &'static str {
    static VALUE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        hash_part(&mut hash, "metric", METRIC_VERSION);
        for (path, content) in SOURCES {
            hash_part(&mut hash, path, content);
        }
        hash.finalize().to_hex().to_string()
    })
}

fn hash_part(hash: &mut blake3::Hasher, label: &str, content: &str) {
    // Length-prefix each part so different label/content boundaries cannot produce the
    // same byte stream.
    for part in [label, content] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    fn rust_files(directory: &Path, root: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, root, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path.strip_prefix(root).unwrap();
                found.push(relative.to_str().unwrap().replace('\\', "/"));
            }
        }
    }

    #[test]
    fn fingerprint_covers_every_source_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut expected = vec!["Cargo.lock".to_owned()];
        rust_files(&root.join("src"), root, &mut expected);
        expected.sort();
        let listed: Vec<_> = SOURCES.iter().map(|(path, _)| *path).collect();
        assert_eq!(listed, expected);
    }
}
