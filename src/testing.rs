//! Test helpers that analyze in-memory sources through the same code paths as a scan.

use crate::{
    analyze::Worker,
    language::Language,
    model::{Analysis, Function, Report, ScanScope},
    path::RelativePath,
    report::{ScannedFile, assemble},
};
use std::path::Path;

/// Analyzes one source, panicking if it cannot be analyzed.
pub fn analysis(language: Language, source: &str) -> Analysis {
    Worker::default()
        .analyze(source, language)
        .unwrap_or_else(|error| panic!("{language} source failed: {error}\n{source}"))
}

/// The first function in a source.
pub fn function(language: Language, source: &str) -> Function {
    analysis(language, source)
        .functions
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no function in {language} source\n{source}"))
}

/// A report for in-memory files, each named by a path whose extension picks its language.
pub fn report(files: &[(&str, &str)]) -> Report {
    let mut files: Vec<_> = files
        .iter()
        .map(|&(path, source)| (Path::new(path), source))
        .collect();
    // Scans order files by path.
    files.sort_by_key(|(path, _)| *path);
    let scanned = files
        .into_iter()
        .map(|(path, source)| {
            let language = Language::detect(path).expect("test paths have supported extensions");
            ScannedFile {
                path: RelativePath::new(path).expect("test paths are UTF-8"),
                language,
                analysis: analysis(language, source),
            }
        })
        .collect();
    let scope = ScanScope::Directory {
        path: RelativePath::lossy(Path::new(".")),
    };
    assemble(scanned, vec![], scope)
}
