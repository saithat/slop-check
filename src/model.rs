//! Analysis facts and the report types written as JSON.

use crate::{
    language::Language,
    lines::{CodeLines, LineSpan},
    path::RelativePath,
};
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;

/// The fixed measurement policy, recorded in every report.
///
/// Keeping the policy out of the CLI makes reports from one version directly comparable,
/// while the report still exposes every threshold that produced it.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub cyclomatic_threshold: u32,
    pub cognitive_threshold: u32,
    pub parameter_threshold: u32,
    pub min_clone_lines: u32,
    pub min_clone_tokens: u32,
    pub min_tuple_return_elements: u32,
    pub min_dispatch_branches: u32,
}

pub const POLICY: Policy = Policy {
    cyclomatic_threshold: 15,
    cognitive_threshold: 10,
    parameter_threshold: 5,
    min_clone_lines: 5,
    min_clone_tokens: 50,
    min_tuple_return_elements: 3,
    min_dispatch_branches: 3,
};

/// A structural rule from the versioned `structural/3` rule pack.
///
/// Variants are declared in the order of their codes, which is the order findings on
/// one line are reported in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    ChainedTupleIndex,
    EmptyCatch,
    EqualityDispatchChain,
    SelfAssignment,
    UnreachableStatement,
    UnreachableWildcardArm,
    WideTupleReturn,
}

impl Rule {
    pub const ALL: [Self; 7] = [
        Self::ChainedTupleIndex,
        Self::EmptyCatch,
        Self::EqualityDispatchChain,
        Self::SelfAssignment,
        Self::UnreachableStatement,
        Self::UnreachableWildcardArm,
        Self::WideTupleReturn,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::ChainedTupleIndex => "chained-tuple-index",
            Self::EmptyCatch => "empty-catch",
            Self::EqualityDispatchChain => "equality-dispatch-chain",
            Self::SelfAssignment => "self-assignment",
            Self::UnreachableStatement => "unreachable-statement",
            Self::UnreachableWildcardArm => "unreachable-wildcard-arm",
            Self::WideTupleReturn => "wide-tuple-return",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ChainedTupleIndex => "tuple position read through another tuple position",
            Self::EmptyCatch => "exception handler with no statements",
            Self::EqualityDispatchChain => "if/else-if chain comparing one value against constants",
            Self::SelfAssignment => "variable assigned to itself",
            Self::UnreachableStatement => "statement after an unconditional jump",
            Self::UnreachableWildcardArm => "wildcard match arm that panics",
            Self::WideTupleReturn => "unlabeled tuple return type",
        }
    }

    /// The languages this rule checks. Findings in any other language are discarded.
    pub fn languages(self) -> &'static [Language] {
        match self {
            Self::ChainedTupleIndex | Self::UnreachableWildcardArm => &[Language::Rust],
            Self::EmptyCatch => &[
                Language::Python,
                Language::Javascript,
                Language::Typescript,
                Language::Tsx,
            ],
            Self::SelfAssignment => &[
                Language::Python,
                Language::Javascript,
                Language::Typescript,
                Language::Tsx,
                Language::Rust,
            ],
            Self::WideTupleReturn => &[
                Language::Python,
                Language::Typescript,
                Language::Tsx,
                Language::Rust,
            ],
            Self::EqualityDispatchChain | Self::UnreachableStatement => &Language::ALL,
        }
    }

    pub fn applies_to(self, language: Language) -> bool {
        self.languages().contains(&language)
    }
}

impl Serialize for Rule {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Function {
    pub name: String,
    #[serde(flatten)]
    pub lines: LineSpan,
    pub sloc: u32,
    pub cyclomatic: u32,
    pub cognitive: u32,
    pub max_nesting: u32,
    pub parameters: u32,
    pub closure: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: Rule,
    #[serde(flatten)]
    pub lines: LineSpan,
    /// The innermost function containing the finding, if any.
    pub function: Option<String>,
}

/// A 128-bit structural fingerprint of a syntax subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub [u8; 16]);

/// A block large enough to report if another block has the same fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub fingerprint: Fingerprint,
    pub lines: LineSpan,
    pub tokens: u32,
}

/// A statically visible import, parsed into the form its language resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    Python(PythonImport),
    /// A JavaScript or TypeScript module specifier from `import`, `export ... from`,
    /// `require()`, or `import()`.
    Specifier(String),
}

/// A Python `import a.b` or `from ..a import b, c`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonImport {
    /// The number of leading dots: 0 for an absolute import.
    pub level: usize,
    /// The dotted module path after the dots.
    pub module: Vec<String>,
    /// Names after `from ... import`, which may be submodules.
    pub names: Vec<String>,
}

impl PythonImport {
    /// Parses module text such as `..a.b` into its level and segments.
    pub fn parse(module: &str, names: Vec<String>) -> Self {
        let path = module.trim_start_matches('.');
        Self {
            level: module.len() - path.len(),
            module: path
                .split('.')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect(),
            names,
        }
    }
}

/// Everything learned from one file's syntax tree.
#[derive(Debug, Default)]
pub struct Analysis {
    pub physical_lines: u32,
    pub code_lines: CodeLines,
    pub functions: Vec<Function>,
    pub findings: Vec<Finding>,
    pub candidates: Vec<Candidate>,
    pub imports: Vec<Import>,
}

/// Per-file detail retained in the report.
#[derive(Debug, Serialize)]
pub struct FileReport {
    pub path: RelativePath,
    pub language: Language,
    pub physical_lines: u32,
    pub sloc: u32,
    pub clone_lines: u32,
    pub flagged_lines: u32,
    pub functions: Vec<Function>,
    pub findings: Vec<Finding>,
}

/// A line span in a particular file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileSpan {
    pub path: RelativePath,
    #[serde(flatten)]
    pub lines: LineSpan,
}

#[derive(Debug, Serialize)]
pub struct CloneGroup {
    pub tokens: u32,
    pub locations: Vec<FileSpan>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub files: u64,
    pub sloc: u64,
    pub functions: u64,
    pub closures: u64,
    pub max_nesting: u32,
    pub cyclomatic: ComplexitySummary,
    pub cognitive: ComplexitySummary,
    pub parameters: ParameterSummary,
    pub clone_lines: u64,
    pub duplication: f64,
    pub rule_findings: u64,
    pub flagged_lines: u64,
    pub flagged_coverage: f64,
}

/// One complexity metric over a population of functions.
///
/// A function's mass is its complexity times the square root of its SLOC. Erosion is the
/// share of mass in functions above the policy threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComplexitySummary {
    pub max: u32,
    pub high_functions: u64,
    pub mass: f64,
    pub high_mass: f64,
    pub erosion: f64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ParameterSummary {
    pub max: u32,
    pub high_functions: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Dependencies {
    pub analyzed_files: u64,
    pub resolved_edges: Vec<Edge>,
    /// Strongly connected components with more than one file, each sorted by path.
    pub cycles: Vec<Vec<RelativePath>>,
    pub unresolved_imports: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: RelativePath,
    pub to: RelativePath,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    /// The file concerned, when the diagnostic is about one file.
    pub path: Option<RelativePath>,
    /// The first line concerned, when the diagnostic points into a file.
    pub line: Option<u32>,
    pub message: String,
}

/// Why part of a scan is missing. Any diagnostic marks the report incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticCode {
    NoSupportedFiles,
    DiscoveryError,
    NonUtf8Path,
    ReadError,
    FileTooLarge,
    AllocationError,
    NotUtf8,
    SyntaxError,
    ParseAborted,
}

impl DiagnosticCode {
    #[cfg(test)]
    pub const ALL: [Self; 9] = [
        Self::NoSupportedFiles,
        Self::DiscoveryError,
        Self::NonUtf8Path,
        Self::ReadError,
        Self::FileTooLarge,
        Self::AllocationError,
        Self::NotUtf8,
        Self::SyntaxError,
        Self::ParseAborted,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::NoSupportedFiles => "no-supported-files",
            Self::DiscoveryError => "discovery-error",
            Self::NonUtf8Path => "non-utf8-path",
            Self::ReadError => "read-error",
            Self::FileTooLarge => "file-too-large",
            Self::AllocationError => "allocation-error",
            Self::NotUtf8 => "not-utf8",
            Self::SyntaxError => "syntax-error",
            Self::ParseAborted => "parse-aborted",
        }
    }
}

impl Serialize for DiagnosticCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

/// What was scanned, relative to the enclosing repository when there is one, so reports
/// do not depend on the checkout directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ScanScope {
    File { path: RelativePath },
    Directory { path: RelativePath },
}

/// The complete report. Its JSON form, which adds the derived observations, is
/// implemented in `report`.
#[derive(Debug)]
pub struct Report {
    pub schema_version: u32,
    pub tool_version: &'static str,
    pub metric_version: &'static str,
    pub analyzer_fingerprint: &'static str,
    pub scan_scope: ScanScope,
    pub policy: &'static Policy,
    pub complete: bool,
    pub summary: Summary,
    pub by_language: BTreeMap<Language, Summary>,
    pub files: Vec<FileReport>,
    pub clones: Vec<CloneGroup>,
    pub dependencies: Dependencies,
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_declared_in_code_order() {
        assert!(Rule::ALL.windows(2).all(|pair| pair[0] < pair[1]));
        let mut codes = Rule::ALL.map(Rule::code);
        codes.sort_unstable();
        assert_eq!(codes, Rule::ALL.map(Rule::code));
    }

    #[test]
    fn the_schema_lists_every_diagnostic_code() {
        // An exhaustive match fails to compile when a variant is added without ALL.
        for code in DiagnosticCode::ALL {
            match code {
                DiagnosticCode::NoSupportedFiles
                | DiagnosticCode::DiscoveryError
                | DiagnosticCode::NonUtf8Path
                | DiagnosticCode::ReadError
                | DiagnosticCode::FileTooLarge
                | DiagnosticCode::AllocationError
                | DiagnosticCode::NotUtf8
                | DiagnosticCode::SyntaxError
                | DiagnosticCode::ParseAborted => {}
            }
        }
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../docs/report.schema.json")).unwrap();
        let listed = &schema["$defs"]["diagnostic"]["properties"]["code"]["enum"];
        assert_eq!(
            *listed,
            serde_json::json!(DiagnosticCode::ALL.map(DiagnosticCode::code))
        );
    }

    #[test]
    fn python_imports_split_level_and_segments() {
        let import = PythonImport::parse("..pkg.module", vec!["name".into()]);
        assert_eq!(import.level, 2);
        assert_eq!(import.module, ["pkg", "module"]);
        assert_eq!(
            PythonImport::parse(".", vec![]).module,
            Vec::<String>::new()
        );
    }
}
