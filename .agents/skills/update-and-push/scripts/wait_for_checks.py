#!/usr/bin/env python3
"""Wait for all GitHub Actions runs for a commit and fail if any did not pass."""
import argparse
import json
import subprocess
import sys
import time

ACCEPTABLE = {"success", "skipped", "neutral"}


def gh_json(arguments):
    output = subprocess.check_output(["gh", *arguments], text=True)
    return json.loads(output)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("commit", nargs="?", help="Commit SHA (defaults to HEAD)")
    parser.add_argument("--timeout", type=int, default=3600, help="Overall timeout in seconds")
    parser.add_argument("--interval", type=int, default=10, help="Polling interval in seconds")
    parser.add_argument(
        "--settle",
        type=int,
        default=15,
        help="Seconds to keep polling after all known runs complete, to catch late-triggered workflows",
    )
    parser.add_argument(
        "--no-runs-timeout",
        type=int,
        default=300,
        help="Fail if no workflow run appears within this many seconds",
    )
    args = parser.parse_args()

    commit = args.commit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], text=True
    ).strip()
    started = time.monotonic()
    waiting_for_first_run = True
    completed_since = None
    seen = {}

    while True:
        elapsed = time.monotonic() - started
        if elapsed > args.timeout:
            print(f"error: timed out after {args.timeout}s waiting for workflows", file=sys.stderr)
            return 2
        try:
            runs = gh_json(
                [
                    "run",
                    "list",
                    "--commit",
                    commit,
                    "--limit",
                    "100",
                    "--json",
                    "databaseId,workflowName,status,conclusion,url",
                ]
            )
        except FileNotFoundError:
            print("error: gh is not installed", file=sys.stderr)
            return 2
        except subprocess.CalledProcessError as error:
            print(f"error: gh run list failed ({error})", file=sys.stderr)
            return 2

        if not runs:
            if waiting_for_first_run:
                started_without_runs = time.monotonic()
                waiting_for_first_run = False
            elif time.monotonic() - started_without_runs > args.no_runs_timeout:
                print(f"error: no workflow runs appeared for {commit}", file=sys.stderr)
                return 2
            print(f"No workflow runs for {commit} yet; waiting...")
            time.sleep(args.interval)
            continue

        waiting_for_first_run = True
        for run in runs:
            seen[run["databaseId"]] = run
        pending = [run for run in runs if run["status"] != "completed"]
        if not pending:
            if completed_since is None:
                completed_since = time.monotonic()
            if time.monotonic() - completed_since >= args.settle:
                break
            print(f"All known runs complete; watching for late workflows for {args.settle}s...")
            time.sleep(args.interval)
            continue

        completed_since = None
        print(f"{len(pending)} workflow run(s) still active...")
        time.sleep(args.interval)

    failed = [
        run for run in seen.values() if (run.get("conclusion") or "") not in ACCEPTABLE
    ]
    for run in seen.values():
        print(f"{run['workflowName']}: {run['conclusion']} ({run['url']})")
    if failed:
        print(f"{len(failed)} workflow run(s) did not pass", file=sys.stderr)
        return 1
    print(f"All {len(seen)} workflow run(s) passed for {commit}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
