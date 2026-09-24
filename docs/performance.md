# Performance

Measured on September 13, 2026, on Linux x86-64 with an AMD Ryzen 9
7950X3D (16 cores, 32 logical CPUs). The benchmark used Rust 1.85.0 and the
release profile in `Cargo.toml`: thin LTO, one code-generation unit, and a
stripped binary.

The benchmark generates a temporary corpus evenly split across Python,
JavaScript, TypeScript, Rust, and Go. Each file contains 20 functions with
nested control flow and repeated structural blocks. This exercises discovery,
parsing, complexity metrics, clone grouping, report construction, and output.

| Corpus | Source | Physical lines | SLOC |
| --- | ---: | ---: | ---: |
| 1,000 files | 6,371,345 bytes | 303,400 | 284,200 |
| 5,000 files | 31,945,540 bytes | 1,517,000 | 1,421,000 |
| 20,000 files | 128,048,763 bytes | 6,068,000 | 5,684,000 |

The two rows below are the complete public output modes. Times include process
startup and writing or capturing the report. They are medians of five runs for
1,000 files and three runs for both larger corpora.

| Mode | 1,000 files | Output | 5,000 files | Output | 20,000 files | Output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Human report on stdout | 72.827 ms (72.271–74.983) | 569,335 bytes | 348.009 ms (339.837–354.251) | 2,956,555 bytes | 1,369.793 ms (1,369.470–1,382.887) | 12,228,685 bytes |
| `--json FILE` | 78.319 ms (76.595–79.569) | 5,082,112 bytes | 366.098 ms (362.574–368.012) | 25,679,735 bytes | 1,455.801 ms (1,455.572–1,471.703) | 103,780,882 bytes |

The benchmark verifies that each mode produces byte-identical output across
its repeated runs. It leaves operating-system filesystem caches in their
normal state and lets `slop-check` choose parallelism automatically. The JSON
row measures the report file itself; the short confirmation written to stderr
is not included in the output size. Results will vary with hardware, file
sizes, language mix, filesystem state, and the number of observations emitted.

Separate `/usr/bin/time -v` runs over the retained 20,000-file corpus measured
174,788 KiB (170.7 MiB) peak resident memory for the human mode and 174,304 KiB
(170.2 MiB) for JSON. The corpus deliberately contains repeated structures, so
the 103.8 MB JSON report includes 320,000 duplicate-block locations. Report
detail and output volume can therefore become the limiting costs on highly
repetitive repositories even when parsing time remains close to linear.

The measured release binary is 8,560,616 bytes (8.16 MiB) and bundles the
parsers for all supported languages. Its SHA-256 is:

```text
a8331c22268c78a8672da61376c57a7c47d71b3a34a94057d5ecf33699fd6b0b
```

The analyzer fingerprint in all three benchmark reports is:

```text
3170a016bab281b7aca12f22dad2873d4290deb1e15357f4fe6763f00abd018f
```

## Ownership cleanup comparison

On September 14, the preceding release binary and the ownership cleanup were
compared on the same 20,000-file corpus described above. Each output mode ran
three times per binary, alternating which binary ran first. Elapsed time includes
process startup and output; peak resident memory comes from `/usr/bin/time`.
The table reports medians, with elapsed-time ranges in parentheses.

| Mode | Before | After | Peak memory before | Peak memory after |
| --- | ---: | ---: | ---: | ---: |
| Human stdout | 1,468.281 ms (1,444.892–1,487.714) | 1,426.806 ms (1,370.975–1,478.836) | 169.8 MiB | 154.4 MiB |
| JSON file | 1,518.861 ms (1,517.593–1,595.749) | 1,522.940 ms (1,507.364–1,586.745) | 169.5 MiB | 155.0 MiB |

Peak memory fell about 9%; elapsed times remained in the same range. Observations
now borrow existing names, paths, clone locations, and cycle members while writing
output. Coverage calculations reuse their range buffer, and human location
formatting writes directly to the output buffer.

Every repeated output was byte-identical for its binary. Outputs also matched
between binaries after replacing only the JSON analyzer fingerprint. The human
report contained 12,228,685 bytes and JSON contained 103,780,877 bytes for this
scan scope. No metrics, evidence, field order, or output limits changed.

The new release binary is 8,564,712 bytes. Its SHA-256 is
`c48527a9f57bc2fab3a82594c96b084646c36d8c6cb6031f54a2951ed2ee270e`,
and its analyzer fingerprint is
`478594625ffb19a2696424c8af162ee9ba4738ea587d89e19f7bc561cf5cc9af`.

## Large files

Measured on September 15 with the same machine and Rust 1.85.0 release profile.
The baseline is the ownership-cleanup binary above. Times include process startup
and JSON output, with GNU time collecting peak resident memory. Each row uses
three runs per binary; baseline samples were retained while the final scheduling
change was checked. Filesystem caches were not flushed. Values are medians, with
elapsed-time ranges in parentheses.

| Input | Before | After | Peak RSS before / after |
| --- | ---: | ---: | ---: |
| 33.26 MB JS, 100,000 routines, 1.5 million SLOC | 7,462.82 ms (7,274.67–7,679.40) | 5,534.87 ms (5,499.23–5,648.95) | 992.5 / 992.8 MiB |
| 2.17 MB JS, one function, 100,000 body statements | 718.16 ms (689.81–739.24) | 576.05 ms (571.17–582.55) | 124.0 / 124.2 MiB |
| 1.96 MB minified JS, 10,000 routines on one line | 700.78 ms (685.70–710.71) | 502.12 ms (499.80–508.90) | 98.2 / 99.0 MiB |
| 50,027-byte JS expression with 10,000 `&&` operands | 2,430.07 ms (2,411.46–2,470.39) | 27.28 ms (26.97–27.86) | 18.2 / 17.8 MiB |
| 1,000 mixed-language files, 6.37 MB, 20,000 routines | 76.15 ms (75.37–78.82) | 71.27 ms (70.76–72.05) | 20.7 / 20.2 MiB |

Traversal now obtains parent, grandparent, and sibling context from its existing
stack. This removes repeated tree searches, especially costly in deep expressions.
Parsing and traversal within one file remain sequential, and the complete syntax
tree remains in memory. The roughly 993 MiB peak for a 33 MB source file illustrates
why source bytes alone are not a prediction of memory consumption.

For concurrent large files, the scanner groups similar source sizes into bands.
Sizes round up to powers of two; each band's worker count is capped by a combined
64 MiB input estimate and available CPU parallelism. Bands run largest first in
separate scoped pools, which join their workers before the next band. This lets
large files reuse a small number of parser workers and allocator arenas. A file
larger than the allowance still runs, by itself. Results return to path order
before aggregation, so scheduling does not change the report.

A fresh one-run comparison of four 33.26 MB files (133,026,516 bytes total,
400,000 routines, 6 million SLOC) measured **3,956.2 → 2,021.4 MiB peak RSS**.
The memory reduction costs throughput: elapsed time was **7.489 → 10.694 seconds**,
about 43% longer. This is an automatic memory/throughput tradeoff, not a hard RAM
limit: syntax-tree expansion, allocator retention, file growth, and retained report
data can exceed the source-size estimate.

All final comparison outputs matched their baseline reports byte-for-byte after
replacing only the analyzer fingerprint. Regression tests also cover a real
40,000-routine source file, a file over the scheduling allowance, deep expressions,
and canonical ordering across size bands. Tree-sitter's 32-bit byte offsets require
each file to be smaller than 4 GiB; larger inputs now produce an explicit diagnostic
before reading rather than risking truncation. There is no repository-size cap.

The measured release binary is 8,593,384 bytes, with SHA-256
`5f7364706bc870cc0fdcf95fa0be8c161946374f89b8fdc774467fc8acfa6002`.
Its analyzer fingerprint is
`51ac4ff4b65be247a70a4fbd741e79791420f6899abd7a7509155e8d4d927887`.

## Type-driven refactor comparison

Measured on September 24, 2026, on the same machine. Both binaries were built
with Rust 1.98.1 and the release profile. The baseline is the `metrics/4`
release; the new binary is `metrics/5`, which classifies each grammar's node kinds
once by kind id instead of comparing kind strings at every node, and adds four
structural rules and the parameter observation. Values are medians, with
elapsed-time ranges in parentheses.

| Corpus and mode | Before | After |
| --- | ---: | ---: |
| 1,000 files, human stdout (5 runs) | 66.273 ms (64.136–67.315) | 54.604 ms (53.391–55.344) |
| 1,000 files, `--json FILE` (5 runs) | 69.726 ms (69.217–70.197) | 59.639 ms (58.416–60.418) |
| 5,000 files, human stdout (5 runs) | 300.145 ms (297.587–315.225) | 253.513 ms (252.713–255.142) |
| 5,000 files, `--json FILE` (5 runs) | 329.788 ms (328.624–334.194) | 281.848 ms (274.193–292.956) |

`scripts/benchmark_large.py --functions 10000 --runs 3` measured the single-file
cases below. Peak resident memory stayed within 1 MiB of the baseline in every
case.

| Case | Before | After |
| --- | ---: | ---: |
| Python, 10,000 routines | 465.7 ms | 401.7 ms |
| JavaScript, 10,000 routines | 576.7 ms | 431.3 ms |
| TypeScript, 10,000 routines | 620.9 ms | 494.2 ms |
| Rust, 10,000 routines | 572.3 ms | 484.7 ms |
| Go, 10,000 routines | 532.9 ms | 415.2 ms |
| One huge JavaScript function | 575.6 ms | 463.7 ms |
| Minified JavaScript | 516.6 ms | 400.1 ms |
| 10,000-operand boolean chain | 27.2 ms | 24.4 ms |

## Reproduce

```sh
cargo build --release --locked
python3 scripts/benchmark.py --files 1000 --functions 20 --runs 5 \
  --output bench-results/local-1000.json
python3 scripts/benchmark.py --files 5000 --functions 20 --runs 3 \
  --output bench-results/local-5000.json
python3 scripts/benchmark.py --files 20000 --functions 20 --runs 3 \
  --output bench-results/local-20000.json
python3 scripts/benchmark_large.py --functions 10000 --runs 3 \
  --output bench-results/local-large-files.json
```

The driver is Python, but the measured analyzer is the release Rust binary.
The earlier tables above measured a human report on stdout and `--json FILE`.
The tool now has one pretty-printed JSON report, so `benchmark.py` measures it
on stdout and with `--output FILE`. The generated corpus and reports live in a temporary directory and are removed
after each benchmark. The saved benchmark result records the platform, CPU
count, corpus dimensions, timing range, output size, binary hash, and analyzer
fingerprint.

The large-file driver scans each case separately: many routines in all five
languages, one huge function, minified JavaScript, and a deep boolean expression.
It checks complete reports, routine counts, and repeated JSON identity, and records
peak RSS where GNU time is available. `--functions 100000` increases the JS routine
case to the 33 MB scale above and scales the other stress cases as well. Benchmark
driver options do not add options to the installed CLI.
