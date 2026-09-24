//! The command-line contract: arguments, output channels, exit status, file discovery,
//! and the report's content. Metric definitions are tested in the library, and the
//! report's shape in `tests/schema.rs`.

use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const LEGACY_OPTIONS: &[&str] = &[
    "--json",
    "--format",
    "--cache",
    "--no-cache",
    "--threads",
    "--timings",
    "--trace",
    "--full",
    "--pretty",
    "--cc-threshold",
    "--cognitive-threshold",
    "--language",
    "--exclude",
    "--min-clone-lines",
    "--min-clone-tokens",
    "--max-file-bytes",
    "--exact-clones",
];

fn write(root: &Path, path: &str, source: impl AsRef<[u8]>) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, source).unwrap();
}

fn command(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slop-check"))
        .arg(root)
        .args(arguments)
        .output()
        .unwrap()
}

/// A scan that printed its report: the process output, the parsed report, and its bytes.
struct Run {
    output: Output,
    report: Value,
    bytes: Vec<u8>,
}

fn run(root: &Path) -> Run {
    let output = command(root, &[]);
    let bytes = output.stdout.clone();
    let report = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "stdout is not a JSON report: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    Run {
        output,
        report,
        bytes,
    }
}

/// The report of a scan that must complete.
fn report(root: &Path) -> Value {
    let Run { output, report, .. } = run(root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    report
}

/// A function of ten chained additions: large enough to be a reported clone.
fn python_pipeline(function: &str, input: &str, prefix: &str) -> String {
    let mut source = format!("def {function}({input}):\n");
    let mut previous = input.to_owned();
    for index in 0..10 {
        let next = format!("{prefix}{index}");
        writeln!(source, "    {next} = {previous} + {}", index + 1).unwrap();
        previous = next;
    }
    writeln!(source, "    return {previous}").unwrap();
    source
}

fn many_findings(function_count: usize) -> String {
    let mut source = String::new();
    for index in 0..function_count {
        writeln!(
            source,
            "def function_{index}(value):\n    value = value\n    return value"
        )
        .unwrap();
    }
    source
}

#[test]
fn help_describes_only_the_minimal_interface() {
    let output = Command::new(env!("CARGO_BIN_EXE_slop-check"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("[PATH]"));
    assert!(help.contains("-o, --output <FILE>"));
    let option_lines: Vec<_> = help
        .lines()
        .filter(|line| line.trim_start().starts_with('-'))
        .collect();
    assert_eq!(option_lines.len(), 3, "unexpected public option:\n{help}");
    assert!(option_lines.iter().any(|line| line.contains("--output")));
    assert!(option_lines.iter().any(|line| line.contains("--help")));
    assert!(option_lines.iter().any(|line| line.contains("--version")));
    for option in LEGACY_OPTIONS {
        assert!(!help.contains(option), "legacy option exposed: {option}");
    }
}

#[test]
fn output_requires_a_destination_and_legacy_options_are_rejected() {
    let root = TempDir::new().unwrap();

    let missing_destination = command(root.path(), &["--output"]);
    assert_eq!(missing_destination.status.code(), Some(2));
    assert!(missing_destination.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&missing_destination.stderr)
            .contains("a value is required for '--output <FILE>'")
    );

    for option in LEGACY_OPTIONS {
        let output = command(root.path(), &[*option]);
        assert_eq!(output.status.code(), Some(2), "accepted {option}");
        assert!(output.stdout.is_empty(), "wrote stdout for {option}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unexpected argument"),
            "unexpected error for {option}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_scan_prints_one_pretty_printed_json_report_and_writes_nothing() {
    let root = TempDir::new().unwrap();
    write(
        root.path(),
        "a.py",
        "def choose(value):\n    value = value\n    return value\n",
    );

    let Run { output, bytes, .. } = run(root.path());

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(bytes.starts_with(b"{\n  \"schema_version\": 6,\n"));
    assert!(bytes.ends_with(b"}\n"));
    let entries: Vec<_> = fs::read_dir(root.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn output_writes_the_same_report_to_a_file() {
    let root = TempDir::new().unwrap();
    write(root.path(), "a.py", "def f():\n    return 1\n");
    let destination = TempDir::new().unwrap();
    let path = destination.path().join("report.json");

    let output = command(root.path(), &["--output", path.to_str().unwrap()]);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrote report to"));
    assert_eq!(fs::read(&path).unwrap(), run(root.path()).bytes);

    let short = destination.path().join("short.json");
    assert!(
        command(root.path(), &["-o", short.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(fs::read(short).unwrap(), fs::read(path).unwrap());
}

#[test]
fn every_observation_and_location_is_reported() {
    let root = TempDir::new().unwrap();
    let mut source = String::from("def findings():\n");
    for index in 0..12 {
        writeln!(source, "    value_{index} = value_{index}").unwrap();
    }
    source.push_str("    return 0\n");
    write(root.path(), "a.py", &source);
    for (path, function, input, prefix) in [
        ("b.py", "alpha", "input", "step"),
        ("c.py", "beta", "value", "result"),
        ("d.py", "gamma", "item", "stage"),
        ("e.py", "delta", "number", "part"),
    ] {
        write(root.path(), path, python_pipeline(function, input, prefix));
    }

    let r = report(root.path());

    let observations = r["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 13);
    let lines: Vec<_> = observations[..12]
        .iter()
        .map(|observation| observation["locations"][0]["start_line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines, (2..=13).collect::<Vec<_>>());
    assert!(
        observations[..12]
            .iter()
            .all(|observation| observation["symbol"] == "findings")
    );
    let clone = &observations[12];
    assert_eq!(clone["code"], "duplicate-block");
    let paths: Vec<_> = clone["locations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|location| location["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["b.py", "c.py", "d.py", "e.py"]);
}

#[test]
fn an_early_closed_output_pipe_is_not_an_analysis_failure() {
    let root = TempDir::new().unwrap();
    write(root.path(), "large.py", many_findings(2_000));
    let mut child = Command::new(env!("CARGO_BIN_EXE_slop-check"))
        .arg(root.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn an_early_closed_output_pipe_preserves_incomplete_scan_status() {
    let root = TempDir::new().unwrap();
    write(root.path(), "bad.py", "def broken(\n");
    write(root.path(), "large.py", many_findings(2_000));
    let mut child = Command::new(env!("CARGO_BIN_EXE_slop-check"))
        .arg(root.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert_eq!(child.wait().unwrap().code(), Some(2));
}

#[test]
fn output_write_failures_are_errors() {
    let root = TempDir::new().unwrap();
    write(root.path(), "a.py", "x = 1\n");
    let path = root.path().join("missing/report.json");
    let output = command(root.path(), &["--output", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("write"));
}

#[test]
fn report_contents_are_complete_and_stable() {
    let root = TempDir::new().unwrap();
    let mut complex = String::from("def branch(values):\n");
    for index in 0..11 {
        writeln!(complex, "    if values[{index}]: return {index}").unwrap();
    }
    complex.push_str("    return -1\n");
    write(root.path(), "complex.py", &complex);
    write(
        root.path(),
        "finding.py",
        "def keep(value):\n    value = value\n    return value\n",
    );
    write(
        root.path(),
        "a.py",
        python_pipeline("alpha", "input", "step"),
    );
    write(
        root.path(),
        "b.py",
        python_pipeline("beta", "value", "result"),
    );
    write(root.path(), "cycle/x.py", "from . import y\n");
    write(root.path(), "cycle/y.py", "from . import x\n");

    let Run {
        output,
        report: r,
        bytes,
    } = run(root.path());
    assert!(output.status.success());

    assert_eq!(r["schema_version"], 6);
    assert_eq!(r["tool_version"], VERSION);
    assert_eq!(r["metric_version"], "metrics/5");
    assert_eq!(r["scan_scope"]["kind"], "directory");
    assert_eq!(r["complete"], true);
    assert_eq!(r["diagnostics"], json!([]));
    assert_eq!(r["summary"]["cyclomatic"]["high_functions"], 1);
    assert_eq!(r["summary"]["cognitive"]["max"], 11);
    assert_eq!(
        r["summary"]["parameters"],
        json!({"max": 1, "high_functions": 0})
    );
    assert_eq!(r["by_language"]["python"]["files"], 6);
    assert_eq!(
        r["dependencies"]["resolved_edges"],
        json!([
            {"from": "cycle/x.py", "to": "cycle/y.py"},
            {"from": "cycle/y.py", "to": "cycle/x.py"},
        ])
    );

    let location = json!({"path": "complex.py", "start_line": 1, "end_line": 13});
    let group = &r["clones"][0];
    assert_eq!(
        r["observations"],
        json!([
            {
                "kind": "cyclomatic_complexity",
                "code": "high-cyclomatic-complexity",
                "message": "cyclomatic complexity 12, above reporting threshold 10",
                "symbol": "branch",
                "value": 12,
                "threshold": 10,
                "locations": [location],
            },
            {
                "kind": "cognitive_complexity",
                "code": "high-cognitive-complexity",
                "message": "cognitive complexity 11, above reporting threshold 10",
                "symbol": "branch",
                "value": 11,
                "threshold": 10,
                "locations": [location],
            },
            {
                "kind": "structural_finding",
                "code": "self-assignment",
                "message": "variable assigned to itself",
                "symbol": "keep",
                "value": null,
                "threshold": null,
                "locations": [{"path": "finding.py", "start_line": 2, "end_line": 2}],
            },
            {
                "kind": "duplicate_block",
                "code": "duplicate-block",
                "message": format!("2 matching blocks with {} syntax tokens", group["tokens"]),
                "symbol": null,
                "value": group["tokens"],
                "threshold": 50,
                "locations": group["locations"],
            },
            {
                "kind": "dependency_cycle",
                "code": "dependency-cycle",
                "message": "dependency cycle across 2 files",
                "symbol": null,
                "value": 2,
                "threshold": null,
                "locations": [
                    {"path": "cycle/x.py", "start_line": null, "end_line": null},
                    {"path": "cycle/y.py", "start_line": null, "end_line": null},
                ],
            },
        ])
    );
    let counts: Vec<_> = r["observation_codes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["count"] != 0)
        .map(|entry| {
            (
                entry["code"].as_str().unwrap(),
                entry["count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("high-cyclomatic-complexity", 1),
            ("high-cognitive-complexity", 1),
            ("self-assignment", 1),
            ("duplicate-block", 1),
            ("dependency-cycle", 1),
        ]
    );
    assert_eq!(run(root.path()).bytes, bytes);
}

#[test]
fn ignore_files_and_default_directories_are_respected() {
    let root = TempDir::new().unwrap();
    for path in [
        "keep.py",
        "skip.py",
        "included/a.py",
        ".hidden/keep.py",
        "node_modules/a.js",
        "target/b.rs",
        "vendor/c.go",
        "dist/d.ts",
        "build/e.py",
        ".venv/f.py",
        "__pycache__/g.py",
    ] {
        write(root.path(), path, "# fixture\n");
    }
    write(root.path(), ".gitignore", "skip.py\n");

    let r = report(root.path());

    let paths: Vec<_> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, [".hidden/keep.py", "included/a.py", "keep.py"]);
}

#[test]
fn repeated_scans_produce_byte_identical_reports() {
    let root = TempDir::new().unwrap();
    write(
        root.path(),
        "a.py",
        python_pipeline("alpha", "input", "step"),
    );
    write(
        root.path(),
        "b.py",
        python_pipeline("beta", "value", "result"),
    );
    write(root.path(), "cycle/x.py", "from . import y\n");
    write(root.path(), "cycle/y.py", "from . import x\n");

    let expected = run(root.path()).bytes;
    for _ in 0..8 {
        assert_eq!(run(root.path()).bytes, expected);
    }
}

#[test]
fn incomplete_scans_report_diagnostics_and_fail() {
    let root = TempDir::new().unwrap();
    write(root.path(), "good.py", "x = 1\n");
    write(root.path(), "bad.py", "x = 1\ndef f(\n");
    write(root.path(), "latin1.py", b"x = '\xe9'\n");

    let Run { output, report, .. } = run(root.path());

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["complete"], false);
    assert_eq!(report["summary"]["files"], 1);
    assert_eq!(
        report["diagnostics"],
        json!([
            {
                "code": "syntax-error",
                "path": "bad.py",
                "line": 2,
                "message": "syntax error on line 2; file excluded from metrics",
            },
            {
                "code": "not-utf8",
                "path": "latin1.py",
                "line": 1,
                "message": "source is not UTF-8; the first invalid byte is on line 1",
            },
        ])
    );
}

#[test]
fn empty_scans_are_not_reported_as_clean_and_large_files_are_analyzed() {
    let empty = TempDir::new().unwrap();
    let Run { output, report, .. } = run(empty.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["diagnostics"],
        json!([{
            "code": "no-supported-files",
            "path": null,
            "line": null,
            "message": "no supported source files found",
        }])
    );

    let large = TempDir::new().unwrap();
    let source = format!("# {}\ndef f():\n    return 1\n", "x".repeat(2_097_152));
    write(large.path(), "huge.py", &source);
    let report = self::report(large.path());
    assert_eq!(report["complete"], true);
    assert_eq!(report["summary"]["functions"], 1);
}

#[test]
fn files_beyond_the_parser_address_limit_are_reported_without_reading() {
    let root = TempDir::new().unwrap();
    let oversized = fs::File::create(root.path().join("oversized.py")).unwrap();
    // A sparse file exercises the size check without allocating 4 GiB of source.
    oversized.set_len(u64::from(u32::MAX) + 1).unwrap();
    write(root.path(), "valid.py", "def f():\n    return 1\n");
    let Run {
        output, report: r, ..
    } = run(root.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(r["complete"], false);
    assert_eq!(r["summary"]["functions"], 1);
    assert_eq!(r["diagnostics"][0]["code"], "file-too-large");
    assert_eq!(r["diagnostics"][0]["path"], "oversized.py");
    assert!(
        r["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("4 GiB")
    );
}

#[test]
fn size_bands_preserve_file_order_and_include_files_larger_than_the_allowance() {
    use std::io::Write as _;

    let root = TempDir::new().unwrap();
    write(root.path(), "a.py", "def first():\n    return 1\n");
    let mut large = fs::File::create(root.path().join("z.py")).unwrap();
    large.write_all(b"#").unwrap();
    // Write a long comment in chunks to avoid a second huge buffer in the test.
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..65 {
        large.write_all(&chunk).unwrap();
    }
    large.write_all(b"\ndef last():\n    return 2\n").unwrap();
    drop(large);
    let r = report(root.path());
    assert_eq!(r["summary"]["functions"], 2);
    assert_eq!(r["summary"]["sloc"], 4);
    assert_eq!(r["files"][0]["path"], "a.py");
    assert_eq!(r["files"][1]["path"], "z.py");
    assert_eq!(r["files"][1]["functions"][0]["name"], "last");
}

#[cfg(unix)]
#[test]
fn directory_symlinks_are_not_followed() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    write(root.path(), "keep.py", "x = 1\n");
    write(outside.path(), "outside.py", "y = 2\n");
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    assert_eq!(report(root.path())["summary"]["files"], 1);
}
