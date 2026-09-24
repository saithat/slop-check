//! Cross-file results: clones, coverage, summaries, and observations.

use crate::{
    language::Language,
    model::{Report, Rule},
    observation::{Metric, Observation, ObservationKind, observations},
    testing::report,
};
use std::fmt::Write as _;

/// A function of ten chained additions, the smallest shape the clone thresholds accept.
fn pipeline(
    language: Language,
    function: &str,
    input: &str,
    prefix: &str,
    offset: usize,
) -> String {
    let mut source = match language {
        Language::Python => format!("def {function}({input}):\n"),
        Language::Javascript => format!("function {function}({input}) {{\n"),
        Language::Typescript | Language::Tsx => {
            format!("function {function}({input}: number): number {{\n")
        }
        Language::Rust => format!("fn {function}({input}: i32) -> i32 {{\n"),
        Language::Go => format!("package p\nfunc {function}({input} int) int {{\n"),
    };
    let mut previous = input.to_owned();
    for index in 0..10 {
        let next = format!("{prefix}{index}");
        let value = index + 1 + offset;
        match language {
            Language::Python => writeln!(source, "    {next} = {previous} + {value}"),
            Language::Go => writeln!(source, "    {next} := {previous} + {value}"),
            Language::Javascript | Language::Typescript | Language::Tsx | Language::Rust => {
                writeln!(source, "    let {next} = {previous} + {value};")
            }
        }
        .unwrap();
        previous = next;
    }
    let end = match language {
        Language::Python => format!("    return {previous}\n"),
        Language::Rust => format!("    {previous}\n}}\n"),
        Language::Javascript | Language::Typescript | Language::Tsx | Language::Go => {
            format!("    return {previous};\n}}\n")
        }
    };
    source + &end
}

fn python_pipeline(function: &str, input: &str, prefix: &str) -> String {
    pipeline(Language::Python, function, input, prefix, 0)
}

fn kinds(report: &Report) -> Vec<ObservationKind> {
    observations(report).iter().map(Observation::kind).collect()
}

#[test]
fn comments_do_not_change_clone_fingerprints() {
    let commented = python_pipeline("beta", "value", "result").replacen(
        '\n',
        "\n    # comments are outside the structural fingerprint\n",
        1,
    );
    let report = report(&[
        ("a.py", &python_pipeline("alpha", "input", "step")),
        ("b.py", &commented),
    ]);
    assert_eq!(report.clones.len(), 1);
    assert!(report.summary.clone_lines > 0);
    assert_eq!(report.summary.sloc, 24);
}

#[test]
fn different_literals_are_not_clones() {
    let report = report(&[
        ("a.py", &python_pipeline("alpha", "input", "step")),
        (
            "b.py",
            &pipeline(Language::Python, "beta", "value", "result", 1),
        ),
    ]);
    assert_eq!(report.summary.clone_lines, 0);
}

#[test]
fn normalized_clones_work_in_every_primary_language() {
    let mut files = vec![];
    for (language, extension) in [
        (Language::Python, "py"),
        (Language::Javascript, "js"),
        (Language::Typescript, "ts"),
        (Language::Rust, "rs"),
        (Language::Go, "go"),
    ] {
        files.push((
            format!("a.{extension}"),
            pipeline(language, "alpha", "input", "step", 0),
        ));
        files.push((
            format!("b.{extension}"),
            pipeline(language, "beta", "value", "result", 0),
        ));
    }
    let files: Vec<_> = files
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    let report = report(&files);
    assert_eq!(report.clones.len(), 5);
    assert!(report.clones.iter().all(|group| group.locations.len() == 2));
}

#[test]
fn normalized_clones_preserve_name_reuse() {
    let mut first = String::from("def alpha(x, y):\n");
    let mut second = String::from("def beta(x, y):\n");
    for index in 0..12 {
        writeln!(first, "    x = x + {index}").unwrap();
        let operand = if index == 0 { "y" } else { "x" };
        writeln!(second, "    x = {operand} + {index}").unwrap();
    }
    first.push_str("    return x\n");
    second.push_str("    return x\n");
    assert_eq!(
        report(&[("a.py", &first), ("b.py", &second)])
            .summary
            .clone_lines,
        0
    );
}

#[test]
fn member_names_are_part_of_the_fingerprint() {
    fn calls(header: &str, call: &str, footer: &str) -> String {
        format!("{header}\n{}{footer}\n", format!("    {call}\n").repeat(20))
    }
    for (extension, first, second) in [
        (
            "js",
            calls(
                "function alpha(client) {",
                "client.read();",
                "    return client;\n}",
            ),
            calls(
                "function beta(service) {",
                "service.delete();",
                "    return service;\n}",
            ),
        ),
        (
            "py",
            calls("def alpha(client):", "client.read()", "    return client"),
            calls(
                "def beta(service):",
                "service.delete()",
                "    return service",
            ),
        ),
        (
            "py",
            calls(
                "def alpha(client):",
                "client.send(timeout=1)",
                "    return client",
            ),
            calls(
                "def beta(client):",
                "client.send(retries=1)",
                "    return client",
            ),
        ),
        (
            "rs",
            calls("fn alpha(x: u8) -> u8 {", "Reader::read(x);", "    x\n}"),
            calls("fn beta(x: u8) -> u8 {", "Writer::delete(x);", "    x\n}"),
        ),
    ] {
        let report = report(&[
            (&format!("a.{extension}"), &first),
            (&format!("b.{extension}"), &second),
        ]);
        assert_eq!(report.summary.clone_lines, 0, "{first}");
    }
}

#[test]
fn clone_and_rule_coverage_is_a_union() {
    let with_finding = |function, input: &str, prefix| {
        python_pipeline(function, input, prefix).replacen(
            '\n',
            &format!("\n    {input} = {input}\n"),
            1,
        )
    };
    let report = report(&[
        ("a.py", &with_finding("alpha", "input", "step")),
        ("b.py", &with_finding("beta", "value", "result")),
    ]);
    assert!(report.summary.clone_lines > 0);
    assert_eq!(report.summary.flagged_lines, report.summary.clone_lines);
    assert_eq!(report.summary.rule_findings, 2);
    let kinds = kinds(&report);
    assert!(kinds.contains(&ObservationKind::DuplicateBlock));
    let findings = kinds
        .iter()
        .filter(|kind| **kind == ObservationKind::StructuralFinding);
    assert_eq!(findings.count(), 2);
}

#[test]
fn complexity_summary_matches_measured_mass() {
    let mut source = String::from("def small():\n    return 0\n\ndef branch(values):\n");
    for index in 0..11 {
        writeln!(source, "    if values[{index}]: return {index}").unwrap();
    }
    source.push_str("    return -1\n");
    let report = report(&[("a.py", &source)]);

    let high_mass = 12.0 * 13_f64.sqrt();
    let cyclomatic = &report.summary.cyclomatic;
    assert_eq!(cyclomatic.high_functions, 1);
    assert!((cyclomatic.high_mass - high_mass).abs() < 1e-12);
    assert!((cyclomatic.erosion - high_mass / (high_mass + 2_f64.sqrt())).abs() < 1e-12);
    assert_eq!(report.summary.cognitive.high_functions, 1);
    assert_eq!(
        kinds(&report),
        [
            ObservationKind::Metric(Metric::Cyclomatic),
            ObservationKind::Metric(Metric::Cognitive)
        ]
    );
}

#[test]
fn long_parameter_lists_are_observed() {
    let report = report(&[(
        "a.py",
        "def wide(a, b, c, d, e, f):\n    return a\n\ndef narrow(a, b, c, d, e):\n    return a\n",
    )]);
    assert_eq!(report.summary.parameters.max, 6);
    assert_eq!(report.summary.parameters.high_functions, 1);
    let observed = observations(&report);
    let [
        Observation::Metric {
            metric, function, ..
        },
    ] = observed[..]
    else {
        panic!("expected one metric observation, found {observed:?}");
    };
    assert_eq!(metric, Metric::Parameters);
    assert_eq!(function.name, "wide");
    assert_eq!(
        observed[0].message(),
        "parameter count 6, above reporting threshold 5"
    );
}

#[test]
fn observations_follow_kind_order_then_strength() {
    let mut complex = String::from("def weaker(v):\n");
    for index in 0..11 {
        writeln!(complex, "    if v[{index}]: return {index}").unwrap();
    }
    complex.push_str("    return -1\n\ndef stronger(v):\n");
    for index in 0..12 {
        writeln!(complex, "    if v[{index}]: return {index}").unwrap();
    }
    complex.push_str("    return -1\n");
    let report = report(&[
        ("complex.py", &complex),
        (
            "finding.py",
            "def keep(value):\n    value = value\n    return value\n",
        ),
        ("cycle/x.py", "from . import y\n"),
        ("cycle/y.py", "from . import x\n"),
    ]);
    let observed = observations(&report);
    let symbols: Vec<_> = observed.iter().filter_map(Observation::symbol).collect();
    // A finding names the function that contains it.
    assert_eq!(
        symbols,
        ["stronger", "weaker", "stronger", "weaker", "keep"]
    );
    assert_eq!(
        kinds(&report),
        [
            ObservationKind::Metric(Metric::Cyclomatic),
            ObservationKind::Metric(Metric::Cyclomatic),
            ObservationKind::Metric(Metric::Cognitive),
            ObservationKind::Metric(Metric::Cognitive),
            ObservationKind::StructuralFinding,
            ObservationKind::DependencyCycle,
        ]
    );
    assert_eq!(observed[4].code(), Rule::SelfAssignment.code());
}

#[test]
fn large_source_keeps_every_routine_and_finding() {
    let count = 40_000;
    let mut source = String::new();
    for index in 0..count {
        writeln!(
            source,
            "def function_{index}(value):\n    value = value\n    return value"
        )
        .unwrap();
    }
    assert!(source.len() > 2_097_152);
    let report = report(&[("large.py", &source)]);
    assert_eq!(report.summary.functions, count);
    assert_eq!(report.summary.rule_findings, count);
    assert_eq!(report.summary.sloc, count * 3);
    let functions = &report.files[0].functions;
    assert_eq!(functions[0].name, "function_0");
    assert_eq!(functions.last().unwrap().name, "function_39999");
    assert_eq!(functions.last().unwrap().lines.end(), 120_000);
    assert_eq!(observations(&report).len(), 40_000);
}
