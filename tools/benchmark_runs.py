#!/usr/bin/env python3
"""Run arbitrary benchmark commands repeatedly and collect quasar-profile artifacts.

Spec: {"cases": [{"name": "baseline", "command": ["app", "--benchmark-file",
"{output}"], "env": {}}]}. No shell, build, window, or game policy is imposed.
Run under an existing DISPLAY or wrap with xvfb-run when the commands need X11.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time


def run_cases(spec, output, repeats, timeout):
    if not 1 <= repeats <= 30 or not 0 < timeout <= 3600:
        raise ValueError("repeats must be 1..=30 and timeout 1..=3600 seconds")
    cases = spec["cases"]
    if not 1 <= len(cases) <= 32:
        raise ValueError("spec needs 1..=32 cases")
    names = [case["name"] for case in cases]
    if len(set(names)) != len(names) or any(not re.fullmatch(r"[A-Za-z0-9_-]+", name) for name in names):
        raise ValueError("case names must be unique simple identifiers")
    output.mkdir(parents=True, exist_ok=True)
    if (output / "comparison.json").exists():
        raise FileExistsError("refusing to overwrite comparison.json")
    report = {"cases": {}, "repeats": repeats}
    for case in cases:
        command = case["command"]
        if not command or not all(isinstance(arg, str) for arg in command) or not any("{output}" in arg for arg in command):
            raise ValueError("command must be an argument list with an {output} artifact path")
        runs = []
        for repeat in range(repeats):
            artifact = output / f"{case['name']}-{repeat}.json"
            log = artifact.with_suffix(".log")
            if artifact.exists() or log.exists():
                raise FileExistsError(f"refusing to overwrite {artifact} or {log}")
            args = [arg.replace("{output}", str(artifact.resolve())) for arg in command]
            start = time.monotonic()
            with log.open("x") as stream:
                subprocess.run(args, env=dict(os.environ, **case.get("env", {})), stdout=stream, stderr=subprocess.STDOUT, timeout=timeout, check=True)
            summary = json.loads(artifact.read_text())
            if summary.get("window_samples", 0) <= 0:
                raise ValueError(f"{artifact} has no measured frames")
            runs.append({"artifact": artifact.name, "log": log.name, "elapsed_seconds": time.monotonic()-start, "summary": summary})
            print(f"{case['name']} {repeat+1}/{repeats}: CPU p50={summary['cpu_frame']['p50']/1e6:.3f} ms submit={summary['submit']['p50']/1e6:.3f} ms draws={summary['draws']['p50']}", flush=True)
        report["cases"][case["name"]] = {"command": command, "env": case.get("env", {}), "runs": runs}
    with (output / "comparison.json").open("x") as stream:
        json.dump(report, stream, indent=2)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    args = parser.parse_args()
    run_cases(json.loads(args.spec.read_text()), args.output, args.repeats, args.timeout)


if __name__ == "__main__":
    main()
