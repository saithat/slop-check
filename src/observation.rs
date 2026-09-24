//! Observations: the review-ready view of a report's evidence.
//!
//! Observations borrow facts the report already holds, so they are built only when
//! the report is written.

use crate::{
    language::Language,
    lines::LineSpan,
    model::{CloneGroup, FileSpan, Finding, Function, POLICY, Report, Rule},
    path::RelativePath,
};
use serde::{Serialize, Serializer, ser::SerializeStruct};
use std::{cmp::Reverse, slice};

/// A per-function measurement with a reporting threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cyclomatic,
    Cognitive,
    Parameters,
}

impl Metric {
    pub fn value(self, function: &Function) -> u32 {
        match self {
            Self::Cyclomatic => function.cyclomatic,
            Self::Cognitive => function.cognitive,
            Self::Parameters => function.parameters,
        }
    }

    /// Functions strictly above this value are reported.
    pub fn threshold(self) -> u32 {
        match self {
            Self::Cyclomatic => POLICY.cyclomatic_threshold,
            Self::Cognitive => POLICY.cognitive_threshold,
            Self::Parameters => POLICY.parameter_threshold,
        }
    }

    fn kind_name(self) -> &'static str {
        match self {
            Self::Cyclomatic => "cyclomatic_complexity",
            Self::Cognitive => "cognitive_complexity",
            Self::Parameters => "parameter_count",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Cyclomatic => "high-cyclomatic-complexity",
            Self::Cognitive => "high-cognitive-complexity",
            Self::Parameters => "high-parameter-count",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Cyclomatic => {
                "function or closure whose cyclomatic complexity is above the threshold"
            }
            Self::Cognitive => {
                "function or closure whose cognitive complexity is above the threshold"
            }
            Self::Parameters => "function or closure with more parameters than the threshold",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cyclomatic => "cyclomatic complexity",
            Self::Cognitive => "cognitive complexity",
            Self::Parameters => "parameter count",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationKind {
    /// A function or closure above a metric's threshold.
    Metric(Metric),
    StructuralFinding,
    DuplicateBlock,
    DependencyCycle,
}

impl ObservationKind {
    /// Every kind, in report order. [`observations`] walks this list to produce
    /// observations, so a kind missing from it produces none.
    pub const ALL: [Self; 6] = [
        Self::Metric(Metric::Cyclomatic),
        Self::Metric(Metric::Cognitive),
        Self::Metric(Metric::Parameters),
        Self::StructuralFinding,
        Self::DuplicateBlock,
        Self::DependencyCycle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Metric(metric) => metric.kind_name(),
            Self::StructuralFinding => "structural_finding",
            Self::DuplicateBlock => "duplicate_block",
            Self::DependencyCycle => "dependency_cycle",
        }
    }
}

impl Serialize for ObservationKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// A measurement or structural signal for review. Observations never fail a scan.
#[derive(Debug, Clone, Copy)]
pub enum Observation<'a> {
    Metric {
        metric: Metric,
        path: &'a RelativePath,
        function: &'a Function,
    },
    Finding {
        path: &'a RelativePath,
        finding: &'a Finding,
    },
    DuplicateBlock(&'a CloneGroup),
    DependencyCycle(&'a [RelativePath]),
}

impl<'a> Observation<'a> {
    pub fn kind(&self) -> ObservationKind {
        match self {
            Self::Metric { metric, .. } => ObservationKind::Metric(*metric),
            Self::Finding { .. } => ObservationKind::StructuralFinding,
            Self::DuplicateBlock(_) => ObservationKind::DuplicateBlock,
            Self::DependencyCycle(_) => ObservationKind::DependencyCycle,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Metric { metric, .. } => metric.code(),
            Self::Finding { finding, .. } => finding.rule.code(),
            Self::DuplicateBlock(_) => DUPLICATE_BLOCK,
            Self::DependencyCycle(_) => DEPENDENCY_CYCLE,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Metric {
                metric, function, ..
            } => format!(
                "{} {}, above reporting threshold {}",
                metric.label(),
                metric.value(function),
                metric.threshold()
            ),
            Self::Finding { finding, .. } => finding.rule.description().to_owned(),
            Self::DuplicateBlock(group) => format!(
                "{} matching blocks with {} syntax tokens",
                group.locations.len(),
                group.tokens
            ),
            Self::DependencyCycle(files) => {
                format!("dependency cycle across {} files", files.len())
            }
        }
    }

    pub fn symbol(&self) -> Option<&'a str> {
        match self {
            Self::Metric { function, .. } => Some(&function.name),
            Self::Finding { finding, .. } => finding.function.as_deref(),
            Self::DuplicateBlock(_) | Self::DependencyCycle(_) => None,
        }
    }

    pub fn value(&self) -> Option<u32> {
        match self {
            Self::Metric {
                metric, function, ..
            } => Some(metric.value(function)),
            Self::Finding { .. } => None,
            Self::DuplicateBlock(group) => Some(group.tokens),
            Self::DependencyCycle(files) => Some(u32::try_from(files.len()).unwrap_or(u32::MAX)),
        }
    }

    pub fn threshold(&self) -> Option<u32> {
        match self {
            Self::Metric { metric, .. } => Some(metric.threshold()),
            Self::DuplicateBlock(_) => Some(POLICY.min_clone_tokens),
            Self::Finding { .. } | Self::DependencyCycle(_) => None,
        }
    }

    pub fn locations(&self) -> Locations<'a> {
        match *self {
            Self::Metric { path, function, .. } => Locations::One(Some(Location {
                path: path.as_str(),
                lines: Some(function.lines),
            })),
            Self::Finding { path, finding } => Locations::One(Some(Location {
                path: path.as_str(),
                lines: Some(finding.lines),
            })),
            Self::DuplicateBlock(group) => Locations::Spans(group.locations.iter()),
            Self::DependencyCycle(files) => Locations::Files(files.iter()),
        }
    }
}

/// The JSON shape of an observation. Every key is always present; a value that does
/// not apply to the observation's kind is `null`.
#[derive(Serialize)]
struct ObservationJson<'a> {
    kind: ObservationKind,
    code: &'static str,
    message: String,
    symbol: Option<&'a str>,
    value: Option<u32>,
    threshold: Option<u32>,
    #[serde(serialize_with = "serialize_locations")]
    locations: Locations<'a>,
}

impl Serialize for Observation<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ObservationJson {
            kind: self.kind(),
            code: self.code(),
            message: self.message(),
            symbol: self.symbol(),
            value: self.value(),
            threshold: self.threshold(),
            locations: self.locations(),
        }
        .serialize(serializer)
    }
}

fn serialize_locations<S: Serializer>(
    locations: &Locations<'_>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(locations.clone())
}

/// A place in the source. Dependency cycles name whole files, so their locations have
/// no lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location<'a> {
    pub path: &'a str,
    pub lines: Option<LineSpan>,
}

impl Serialize for Location<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut location = serializer.serialize_struct("Location", 3)?;
        location.serialize_field("path", self.path)?;
        location.serialize_field("start_line", &self.lines.map(LineSpan::start))?;
        location.serialize_field("end_line", &self.lines.map(LineSpan::end))?;
        location.end()
    }
}

/// Iterates an observation's locations without copying clone groups or cycles.
#[derive(Debug, Clone)]
pub enum Locations<'a> {
    One(Option<Location<'a>>),
    Spans(slice::Iter<'a, FileSpan>),
    Files(slice::Iter<'a, RelativePath>),
}

impl<'a> Iterator for Locations<'a> {
    type Item = Location<'a>;

    fn next(&mut self) -> Option<Location<'a>> {
        match self {
            Self::One(location) => location.take(),
            Self::Spans(spans) => spans.next().map(|span| Location {
                path: span.path.as_str(),
                lines: Some(span.lines),
            }),
            Self::Files(files) => files.next().map(|path| Location {
                path: path.as_str(),
                lines: None,
            }),
        }
    }
}

const DUPLICATE_BLOCK: &str = "duplicate-block";
const DEPENDENCY_CYCLE: &str = "dependency-cycle";

/// What an observation code means, which languages can produce it, and how many
/// observations in this report carry it.
#[derive(Debug, Serialize)]
pub struct CodeEntry {
    pub code: &'static str,
    pub kind: ObservationKind,
    pub description: &'static str,
    pub languages: Vec<Language>,
    pub count: usize,
}

/// Every code a report can contain, in report order, whether or not any observation
/// carries it.
pub fn code_index(observations: &[Observation<'_>]) -> Vec<CodeEntry> {
    let entry = |kind, code, description, languages: &[Language]| CodeEntry {
        code,
        kind,
        description,
        languages: languages.to_vec(),
        count: observations
            .iter()
            .filter(|observation| observation.code() == code)
            .count(),
    };
    let mut index = vec![];
    for kind in ObservationKind::ALL {
        match kind {
            ObservationKind::Metric(metric) => {
                index.push(entry(
                    kind,
                    metric.code(),
                    metric.description(),
                    &Language::ALL,
                ));
            }
            ObservationKind::StructuralFinding => index.extend(
                Rule::ALL
                    .iter()
                    .map(|rule| entry(kind, rule.code(), rule.description(), rule.languages())),
            ),
            ObservationKind::DuplicateBlock => index.push(entry(
                kind,
                DUPLICATE_BLOCK,
                "blocks with identical syntax after renaming local names",
                &Language::ALL,
            )),
            ObservationKind::DependencyCycle => {
                let languages: Vec<_> = Language::ALL
                    .into_iter()
                    .filter(|language| language.resolves_imports())
                    .collect();
                index.push(entry(
                    kind,
                    DEPENDENCY_CYCLE,
                    "files that import each other, directly or through other files",
                    &languages,
                ));
            }
        }
    }
    index
}

/// Every observation in the report, grouped in [`ObservationKind::ALL`] order.
pub fn observations(report: &Report) -> Vec<Observation<'_>> {
    let mut result = vec![];
    for kind in ObservationKind::ALL {
        match kind {
            ObservationKind::Metric(metric) => add_metric(&mut result, report, metric),
            ObservationKind::StructuralFinding => {
                result.extend(report.files.iter().flat_map(|file| {
                    file.findings.iter().map(|finding| Observation::Finding {
                        path: &file.path,
                        finding,
                    })
                }));
            }
            ObservationKind::DuplicateBlock => {
                result.extend(report.clones.iter().map(Observation::DuplicateBlock));
            }
            ObservationKind::DependencyCycle => {
                result.extend(
                    report
                        .dependencies
                        .cycles
                        .iter()
                        .map(|cycle| Observation::DependencyCycle(cycle)),
                );
            }
        }
    }
    result
}

fn add_metric<'a>(observations: &mut Vec<Observation<'a>>, report: &'a Report, metric: Metric) {
    let mut above: Vec<_> = report
        .files
        .iter()
        .flat_map(|file| {
            file.functions
                .iter()
                .map(move |function| (&file.path, function))
        })
        .filter(|(_, function)| metric.value(function) > metric.threshold())
        .collect();
    // The strongest signal comes first; source position breaks ties.
    above.sort_by_key(|&(path, function)| {
        (
            Reverse(metric.value(function)),
            path,
            function.lines.start(),
        )
    });
    observations.extend(
        above
            .into_iter()
            .map(|(path, function)| Observation::Metric {
                metric,
                path,
                function,
            }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_always_have_line_keys() {
        let location = |lines| {
            serde_json::to_value(Location {
                path: "src/a.rs",
                lines,
            })
            .unwrap()
        };
        assert_eq!(
            location(Some(LineSpan::new(3, 9))),
            serde_json::json!({"path": "src/a.rs", "start_line": 3, "end_line": 9})
        );
        assert_eq!(
            location(None),
            serde_json::json!({"path": "src/a.rs", "start_line": null, "end_line": null})
        );
    }
}
