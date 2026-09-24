//! Turns per-file analyses into the report.

mod clones;
mod summary;
#[cfg(test)]
mod tests;

use crate::{
    dependencies,
    language::Language,
    lines::LineSpan,
    model::{
        Analysis, CloneGroup, Dependencies, Diagnostic, FileReport, POLICY, Policy, Report,
        ScanScope, Summary,
    },
    observation::{CodeEntry, Observation, code_index, observations},
    path::RelativePath,
    version::{METRIC_VERSION, SCHEMA_VERSION, TOOL_VERSION, analyzer_fingerprint},
};
use serde::{Serialize, Serializer};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};
use summary::Totals;

/// A successfully analyzed file.
pub struct ScannedFile {
    pub path: RelativePath,
    pub language: Language,
    pub analysis: Analysis,
}

/// Builds the report. `files` must be in scan order, which keeps file detail, clone
/// locations, and the observations derived from them reproducible.
pub fn assemble(
    files: Vec<ScannedFile>,
    diagnostics: Vec<Diagnostic>,
    scan_scope: ScanScope,
) -> Report {
    // Both cross-file analyses are independent once parsing is complete.
    let (dependencies, clones) =
        rayon::join(|| dependencies::analyze(&files), || clones::group(&files));
    let mut totals = Totals::default();
    let mut by_language = BTreeMap::<Language, Totals>::new();
    let mut reports = Vec::with_capacity(files.len());
    for (file, clone_spans) in files.into_iter().zip(clones.coverage) {
        let report = file_report(file, clone_spans);
        totals.add(&report);
        by_language.entry(report.language).or_default().add(&report);
        reports.push(report);
    }
    Report {
        schema_version: SCHEMA_VERSION,
        tool_version: TOOL_VERSION,
        metric_version: METRIC_VERSION,
        analyzer_fingerprint: analyzer_fingerprint(),
        scan_scope,
        policy: &POLICY,
        complete: diagnostics.is_empty(),
        summary: totals.finish(),
        by_language: by_language
            .into_iter()
            .map(|(language, totals)| (language, totals.finish()))
            .collect(),
        files: reports,
        clones: clones.groups,
        dependencies,
        diagnostics,
    }
}

fn file_report(file: ScannedFile, clone_spans: Vec<LineSpan>) -> FileReport {
    let Analysis {
        physical_lines,
        code_lines,
        functions,
        findings,
        ..
    } = file.analysis;
    // Clone spans are already disjoint; findings may overlap them.
    let clone_lines = clone_spans
        .iter()
        .map(|span| code_lines.count_in(*span))
        .sum();
    let mut flagged = clone_spans;
    flagged.extend(findings.iter().map(|finding| finding.lines));
    FileReport {
        path: file.path,
        language: file.language,
        physical_lines,
        sloc: code_lines.count(),
        clone_lines,
        flagged_lines: code_lines.count_union(&mut flagged),
        functions,
        findings,
    }
}

/// Writes the report as pretty-printed JSON followed by a newline. This is the only
/// output format.
pub fn write_json(out: impl Write, report: &Report) -> io::Result<()> {
    let mut out = io::BufWriter::new(out);
    serde_json::to_writer_pretty(&mut out, report)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// The JSON shape of a report. Fields run from what a reader needs first to the
/// bulkiest detail: identity, completeness and diagnostics, totals, the ranked
/// observations and the index of their codes, and then per-language, per-file, clone,
/// and dependency detail.
#[derive(Serialize)]
struct ReportJson<'a> {
    schema_version: u32,
    tool_version: &'a str,
    metric_version: &'a str,
    analyzer_fingerprint: &'a str,
    scan_scope: &'a ScanScope,
    complete: bool,
    diagnostics: &'a [Diagnostic],
    policy: &'a Policy,
    summary: &'a Summary,
    observations: Vec<Observation<'a>>,
    observation_codes: Vec<CodeEntry>,
    by_language: &'a BTreeMap<Language, Summary>,
    files: &'a [FileReport],
    clones: &'a [CloneGroup],
    dependencies: &'a Dependencies,
}

impl Serialize for Report {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // No `..`: a new report field fails to compile until it is serialized here.
        let Report {
            schema_version,
            tool_version,
            metric_version,
            analyzer_fingerprint,
            scan_scope,
            policy,
            complete,
            summary,
            by_language,
            files,
            clones,
            dependencies,
            diagnostics,
        } = self;
        let observations = observations(self);
        ReportJson {
            schema_version: *schema_version,
            tool_version,
            metric_version,
            analyzer_fingerprint,
            scan_scope,
            complete: *complete,
            diagnostics,
            policy,
            summary,
            observation_codes: code_index(&observations),
            observations,
            by_language,
            files,
            clones,
            dependencies,
        }
        .serialize(serializer)
    }
}
