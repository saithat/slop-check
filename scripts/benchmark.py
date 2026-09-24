#!/usr/bin/env python3
"""Reproducible end-to-end benchmark; Python is only the benchmark driver."""
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

TEMPLATES = {
    "py": """def @NAME@(items):
    total = @VALUE@
    for item in items:
        if item > 0:
            if item % 2 == 0:
                total += item
            else:
                total -= item
        elif item < -10:
            total += 1
    return total
""",
    "js": """function @NAME@(items) {
    let total = @VALUE@;
    for (const item of items) {
        if (item > 0) {
            if (item % 2 === 0) {
                total += item;
            } else {
                total -= item;
            }
        } else if (item < -10) {
            total += 1;
        }
    }
    return total;
}
""",
    "rs": """fn @NAME@(items: &[i32]) -> i32 {
    let mut total = @VALUE@;
    for &item in items {
        if item > 0 {
            if item % 2 == 0 {
                total += item;
            } else {
                total -= item;
            }
        } else if item < -10 {
            total += 1;
        }
    }
    total
}
""",
    "go": """func @NAME@(items []int) int {
    total := @VALUE@
    for _, item := range items {
        if item > 0 {
            if item % 2 == 0 {
                total += item
            } else {
                total -= item
            }
        } else if item < -10 {
            total += 1
        }
    }
    return total
}
""",
}
TEMPLATES["ts"] = TEMPLATES["js"].replace("(items)", "(items: number[]): number")


def generate(root, files, functions):
    paths = []
    for i in range(files):
        extension = ["py", "js", "ts", "rs", "go"][i % 5]
        path = root / f"group_{i // 100}" / f"module_{i}.{extension}"
        path.parent.mkdir(exist_ok=True)
        text = "package bench\n\n" if extension == "go" else ""
        text += "\n".join(
            TEMPLATES[extension].replace("@NAME@", f"process_{i}_{j}")
            .replace("@VALUE@", str((i * functions + j) % 31))
            for j in range(functions)
        )
        path.write_text(text)
        paths.append(path)
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/slop-check"))
    parser.add_argument("--files", type=int, default=1000)
    parser.add_argument("--functions", type=int, default=20)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if min(args.files, args.functions, args.runs) < 1:
        parser.error("files, functions, and runs must be positive")
    binary = args.binary.resolve(strict=True)
    results = {"platform": platform.platform(), "cpu_count": os.cpu_count(),
               "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
               "files": args.files, "functions_per_file": args.functions, "runs": args.runs,
               "note": "Synthetic mixed-language corpus; OS filesystem cache is not flushed. Includes process startup and output serialization."}
    with tempfile.TemporaryDirectory(prefix="slop-check-bench-") as directory:
        workspace = Path(directory)
        root = workspace / "corpus"
        root.mkdir()
        json_path = workspace / "report.json"
        paths = generate(root, args.files, args.functions)
        results["source_bytes"] = sum(p.stat().st_size for p in paths)
        results["physical_lines"] = sum(len(p.read_text().splitlines()) for p in paths)

        def measure(name, arguments, runs, read_output):
            samples = []
            expected = None
            for _ in range(runs):
                json_path.unlink(missing_ok=True)
                start = time.perf_counter()
                run = subprocess.run([str(binary), str(root), *arguments],
                                     check=True, capture_output=True)
                samples.append((time.perf_counter() - start) * 1000)
                output = read_output(run)
                digest = hashlib.sha256(output).hexdigest()
                if expected is None:
                    expected = digest
                elif digest != expected:
                    raise RuntimeError(f"{name}: output changed between runs")
            results[name] = {"median_ms": round(statistics.median(samples), 3),
                             "min_ms": round(min(samples), 3), "max_ms": round(max(samples), 3),
                             "output_bytes": len(output)}
            print(f"{name:20} {statistics.median(samples):9.2f} ms", flush=True)

        measure("stdout", [], args.runs, lambda run: run.stdout)
        measure(
            "output_file",
            ["--output", str(json_path)],
            args.runs,
            lambda _run: json_path.read_bytes(),
        )
        report = json.loads(json_path.read_bytes())
        results["sloc"] = report["summary"]["sloc"]
        results["analyzer_fingerprint"] = report["analyzer_fingerprint"]
    encoded = json.dumps(results, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded)
    else:
        print(encoded)


if __name__ == "__main__":
    main()
