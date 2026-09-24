#!/usr/bin/env python3
"""Reproducible single-file stress cases; includes startup and JSON output."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

from benchmark import TEMPLATES


def cases(functions):
    for extension in ("py", "js", "ts", "rs", "go"):
        source = "package bench\n\n" if extension == "go" else ""
        source += "\n".join(
            TEMPLATES[extension].replace("@NAME@", f"process_{i}")
            .replace("@VALUE@", str(i % 31))
            for i in range(functions)
        )
        yield f"many_functions_{extension}", extension, source, functions
    source = "function huge(items) {\n  let total = 0;\n"
    source += "".join(f"  total += items[{i % 31}];\n" for i in range(functions * 10))
    yield "huge_function_js", "js", source + "  return total;\n}\n", 1
    source = "".join(
        TEMPLATES["js"].replace("@NAME@", f"process_{i}")
        .replace("@VALUE@", str(i % 31))
        for i in range(functions)
    )
    yield "minified_js", "js", "".join(line.strip() for line in source.splitlines()), functions
    chain = " && ".join("x" for _ in range(functions))
    yield "boolean_chain_js", "js", f"function chain(x) {{ return {chain}; }}\n", 1


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def measure(binary, path, workspace, runs, use_time):
    output = workspace / "report.json"
    rss_file = workspace / "rss.txt"
    elapsed, peaks, hashes, failures = [], [], [], []
    for index in range(runs):
        output.unlink(missing_ok=True)
        command = [str(binary), str(path), "--output", str(output)]
        if use_time:
            command = ["/usr/bin/time", "-f", "%M", "-o", str(rss_file), *command]
        start = time.perf_counter()
        run = subprocess.run(command, capture_output=True)
        elapsed.append((time.perf_counter() - start) * 1000)
        if use_time:
            last_line = rss_file.read_text().splitlines()[-1]
            if last_line.isdecimal():
                peaks.append(int(last_line))
        if run.returncode or run.stdout or not output.exists():
            failures.append({"run": index + 1, "exit_code": run.returncode,
                             "stderr": run.stderr.decode(errors="replace"),
                             "stdout": run.stdout.decode(errors="replace"),
                             "report_exists": output.exists()})
            break
        hashes.append(sha256(output))
    result = {"completed_runs": len(hashes), "median_ms": round(statistics.median(elapsed), 3),
              "min_ms": round(min(elapsed), 3), "max_ms": round(max(elapsed), 3),
              "deterministic_json": len(set(hashes)) == 1 if hashes else None,
              "output_sha256": hashes[0] if hashes else None, "failures": failures}
    if peaks:
        result.update(peak_rss_median_kib=statistics.median(peaks), peak_rss_max_kib=max(peaks))
    if hashes and not failures:
        result["output_bytes"] = output.stat().st_size
        with output.open() as stream:
            report = json.load(stream)
        result.update(analyzer_fingerprint=report["analyzer_fingerprint"],
                      complete=report["complete"], sloc=report["summary"]["sloc"],
                      measured_functions=report["summary"]["functions"])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/slop-check"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--functions", type=int, default=10000)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if min(args.runs, args.functions) < 1:
        parser.error("runs and functions must be positive")
    binary = args.binary.resolve(strict=True)
    use_time = False
    if platform.system() == "Linux" and Path("/usr/bin/time").is_file():
        probe = subprocess.run(["/usr/bin/time", "--version"], capture_output=True)
        use_time = b"gnu time" in (probe.stdout + probe.stderr).lower()
    results = {"platform": platform.platform(), "cpu_count": os.cpu_count(),
               "binary_sha256": sha256(binary), "runs": args.runs,
               "functions": args.functions, "peak_rss_available": use_time,
               "note": "Each case scans one file. Cache is not flushed. Timings include "
                       "startup, JSON serialization, and GNU time overhead when available. "
                       "Hashes compare repeated runs within this invocation.", "cases": {}}
    failed = False
    with tempfile.TemporaryDirectory(prefix="slop-check-large-") as directory:
        workspace = Path(directory)
        for name, extension, source, count in cases(args.functions):
            path = workspace / f"{name}.{extension}"
            path.write_text(source, encoding="utf-8")
            result = {"source_bytes": path.stat().st_size,
                      "physical_lines": len(source.splitlines()), "expected_functions": count}
            result.update(measure(binary, path, workspace, args.runs, use_time))
            result["passed"] = (not result["failures"] and result["deterministic_json"] is True
                                and result.get("complete") is True
                                and result.get("measured_functions") == count)
            results["cases"][name] = result
            failed |= not result["passed"]
            print(f"{name:22} {result['median_ms']:10.2f} ms  "
                  f"{'ok' if result['passed'] else 'FAILED'}", flush=True)
            path.unlink()
    encoded = json.dumps(results, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    else:
        print(encoded)
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
