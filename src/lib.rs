//! Fast, deterministic structural code metrics for review by people and coding agents.
//!
//! [`scan()`] discovers files and analyzes each one (`analyze`); `report` aggregates the
//! results into a [`Report`]; and [`write_json`] writes it. The JSON report is the only
//! output, and `docs/report.schema.json` describes it. `syntax` is the only module that
//! names grammar node kinds.

mod analyze;
mod dependencies;
mod language;
mod lines;
mod model;
mod observation;
mod path;
mod report;
mod scan;
mod syntax;
#[cfg(test)]
mod testing;
mod version;

pub use model::Report;
pub use report::write_json;
pub use scan::scan;
