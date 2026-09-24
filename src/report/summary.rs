//! Running totals that become a [`Summary`] once the whole population is known.

use crate::{
    model::{ComplexitySummary, FileReport, ParameterSummary, Summary},
    observation::Metric,
};

/// Counts and masses accumulated file by file. Ratios exist only on the finished
/// [`Summary`], so no ratio can be read before every file is counted.
#[derive(Default)]
pub(super) struct Totals {
    files: u64,
    sloc: u64,
    functions: u64,
    closures: u64,
    max_nesting: u32,
    cyclomatic: MassTotals,
    cognitive: MassTotals,
    parameters: ParameterSummary,
    clone_lines: u64,
    rule_findings: u64,
    flagged_lines: u64,
}

impl Totals {
    pub(super) fn add(&mut self, file: &FileReport) {
        self.files += 1;
        self.sloc += u64::from(file.sloc);
        self.functions += file.functions.len() as u64;
        self.clone_lines += u64::from(file.clone_lines);
        self.flagged_lines += u64::from(file.flagged_lines);
        self.rule_findings += file.findings.len() as u64;
        for function in &file.functions {
            self.closures += u64::from(function.closure);
            self.max_nesting = self.max_nesting.max(function.max_nesting);
            // sqrt(SLOC) gives large functions more weight without letting their raw
            // length dominate.
            let weight = f64::from(function.sloc).sqrt();
            self.cyclomatic.add(Metric::Cyclomatic, function, weight);
            self.cognitive.add(Metric::Cognitive, function, weight);
            let parameters = Metric::Parameters.value(function);
            self.parameters.max = self.parameters.max.max(parameters);
            if parameters > Metric::Parameters.threshold() {
                self.parameters.high_functions += 1;
            }
        }
    }

    pub(super) fn finish(self) -> Summary {
        Summary {
            files: self.files,
            sloc: self.sloc,
            functions: self.functions,
            closures: self.closures,
            max_nesting: self.max_nesting,
            cyclomatic: self.cyclomatic.finish(),
            cognitive: self.cognitive.finish(),
            parameters: self.parameters,
            clone_lines: self.clone_lines,
            duplication: ratio(self.clone_lines as f64, self.sloc as f64),
            rule_findings: self.rule_findings,
            flagged_lines: self.flagged_lines,
            flagged_coverage: ratio(self.flagged_lines as f64, self.sloc as f64),
        }
    }
}

#[derive(Default)]
struct MassTotals {
    max: u32,
    high_functions: u64,
    mass: f64,
    high_mass: f64,
}

impl MassTotals {
    fn add(&mut self, metric: Metric, function: &crate::model::Function, weight: f64) {
        let value = metric.value(function);
        let mass = f64::from(value) * weight;
        self.max = self.max.max(value);
        self.mass += mass;
        if value > metric.threshold() {
            self.high_functions += 1;
            self.high_mass += mass;
        }
    }

    fn finish(self) -> ComplexitySummary {
        ComplexitySummary {
            max: self.max,
            high_functions: self.high_functions,
            mass: self.mass,
            high_mass: self.high_mass,
            erosion: ratio(self.high_mass, self.mass),
        }
    }
}

fn ratio(part: f64, whole: f64) -> f64 {
    if whole == 0.0 { 0.0 } else { part / whole }
}
