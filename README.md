# slop-check

`slop-check` is a fast, local CLI that reports deterministic structural code
metrics. It gives people and coding agents concrete source locations to inspect
without grading the repository or deciding that anything must change.

It supports Python, JavaScript/JSX, TypeScript/TSX, Rust, and Go in one native
binary. It does not need a language runtime, LLM, or external analyzer.

## Install

For a tagged version, download the archive for your platform from
[GitHub Releases](https://github.com/saithat/slop-check/releases), extract the
single `slop-check` executable, and put it on your `PATH`.

From a source checkout:

```sh
cargo install --path . --locked
```

Source installation needs Rust and a C compiler for the bundled syntax parsers.
Prebuilt archives do not.

## Run

```sh
slop-check
slop-check path/to/repository
slop-check path/to/repository --output report.json
```

Every run produces one JSON report. It goes to stdout, or to FILE with
`-o, --output FILE`, in which case stderr confirms the path. There are no other
options.

Repository ignore files and common generated, dependency, and build directories
are respected automatically. The measurement policy is fixed and recorded in
every report so results from the same version and scope are comparable.

## Report

The report is written for programs and coding agents. It is pretty-printed JSON,
byte-identical for the same analyzer and input, and described by
[`docs/report.schema.json`](docs/report.schema.json). Every object always has all
of its keys; a value that does not apply is `null`. Paths are relative to the scan
root with `/` separators, and lines are 1-based and inclusive.

Fields run from what a reader needs first to the bulkiest detail:

| Field | Contents |
| --- | --- |
| `schema_version`, `tool_version`, `metric_version`, `analyzer_fingerprint` | Which report shape and analyzer produced this report |
| `scan_scope`, `complete`, `diagnostics` | What was scanned, and anything that could not be analyzed |
| `policy` | The fixed thresholds |
| `summary` | Repository totals, including per-metric maxima and erosion |
| `observations` | Every observation: its kind, code, function, value, threshold, and locations |
| `observation_codes` | What each code means, which languages produce it, and how often it occurs |
| `by_language`, `files`, `clones`, `dependencies` | Per-language totals, every function and finding, clone groups, and the import graph |

An excerpt:

```json
{
  "schema_version": 6,
  "scan_scope": { "kind": "directory", "path": "sample" },
  "complete": true,
  "diagnostics": [],
  "observations": [
    {
      "kind": "parameter_count",
      "code": "high-parameter-count",
      "message": "parameter count 6, above reporting threshold 5",
      "symbol": "route",
      "value": 6,
      "threshold": 5,
      "locations": [{ "path": "src/route.py", "start_line": 1, "end_line": 8 }]
    },
    {
      "kind": "structural_finding",
      "code": "equality-dispatch-chain",
      "message": "if/else-if chain comparing one value against constants",
      "symbol": "route",
      "value": null,
      "threshold": null,
      "locations": [{ "path": "src/route.py", "start_line": 2, "end_line": 7 }]
    }
  ]
}
```

Observations are measurements for review; they do not require changes. For
example, list the locations of every structural finding with
`slop-check | jq '.observations[] | select(.kind == "structural_finding")'`.

## Measurements

- Source lines and per-routine cyclomatic complexity, cognitive complexity,
  nesting, parameters, and source span.
- Complexity-mass concentration above fixed reporting thresholds.
- Identifier-normalized duplicate syntax blocks, including exact matches.
- Parameter counts above a fixed threshold.
- Structural rules: self-assignment, empty catch, directly unreachable
  statements, unlabeled tuple return types, chained tuple positions, wildcard
  match arms that only panic, and `if`/`else if` chains that dispatch on one
  value.
- Resolvable local dependency edges and cyclic components for Python and the
  JavaScript family.

See [metric definitions](docs/metrics.md) for formulas and limits, and
[performance measurements](docs/performance.md) for benchmark results.

## Behavior

Scanning is read-only. `slop-check` does not execute source code, invoke package
managers or compilers, write into the scanned repository, or use the network.

There is no repository-size cap. Large files are scheduled automatically to
limit simultaneous parser memory use; files over 64 MiB run alone. Parsing still
holds a complete syntax tree in memory. The bundled parser requires individual
files smaller than 4 GiB; larger files produce an explicit diagnostic.

Unreadable files, invalid syntax, non-UTF-8 sources, failed source-buffer
reservations, and discovery failures appear as diagnostics with a specific code;
syntax and encoding diagnostics name the first offending line. Valid files still contribute results, but the
report is marked incomplete so a partial scan cannot look clean. Available memory
remains a practical limit, and not every allocation failure is recoverable.

Exit status `0` means the scan completed. Exit status `2` means the arguments
were invalid, the scan was incomplete, or the command failed. An incomplete scan
still writes its full report, with `complete: false` and its diagnostics. A failed
command, such as a missing scan path, writes no report and explains the error on
stderr. Observations do not change the exit status.
