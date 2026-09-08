#!/usr/bin/env python3
"""Scenario test runner — executes tests in order and generates a report.

Run core tests:       python3 scenarios/run.py
Run core + optional:  python3 scenarios/run.py --include-optional
Run one test:         python3 scenarios/test_containers.py
"""

import argparse
import os
import subprocess
import sys
import time

# Ensure the project root (parent of scenarios/) is on sys.path so that
# test files can do `from scenarios.run import *` regardless of cwd.
_project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if _project_root not in sys.path:
    sys.path.insert(0, _project_root)

# Re-export everything from helpers so `from scenarios.run import *` still works.
from scenarios.helpers import *
from scenarios.helpers import info
from scenarios.report import TestResult, write_report

# ── Scenario execution order ─────────────────────────────────
# Each entry is (test_name, timeout_seconds, optional). Optional scenarios are
# discoverable and runnable directly, but omitted from the default core run.
CREATE_DATASET_SMOKE_TIMEOUT_SECS = 1800

ORDER = [
    ("test_containers", 5, False),
    ("test_basic_balances", 10, False),
    # Allows setup plus five 280s Node attempts and retry delays.
    ("test_create_dataset_smoke", CREATE_DATASET_SMOKE_TIMEOUT_SECS, False),
    ("test_synapse_e2e", 600, False),
    ("test_negative_permissions", 300, False),
    ("test_multi_copy_upload", 600, False),
    ("test_caching_subsystem", 200, False),
    ("test_bulk_add", 600, True),
    ("test_termination_controls", 900, True),
]


def select_scenarios(include_optional, order=ORDER):
    """Return (selected, skipped) entries for an ordered scenario collection."""
    selected = []
    skipped = []
    for scenario in order:
        if scenario[2] and not include_optional:
            skipped.append(scenario)
        else:
            selected.append(scenario)
    return selected, skipped


def _run_single_test(scenario_py_file, name, timeout_sec):
    """Run one scenario file as a subprocess, return a TestResult."""
    info(f"=== {name} (timeout: {timeout_sec}s) ===")
    test_start = time.time()
    process = subprocess.Popen(
        [sys.executable, scenario_py_file],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    timed_out = False
    try:
        stdout, _ = process.communicate(timeout=timeout_sec)
        return_code = process.returncode
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, _ = process.communicate()
        timed_out, return_code = True, -1
    except Exception as e:
        process.kill()
        process.wait()
        stdout, return_code = f"[ERROR] Exception during test execution: {e}", -1

    log_lines = stdout.splitlines()
    if timed_out:
        log_lines.append(f"[TIMEOUT] Test '{name}' exceeded {timeout_sec}s limit")
    for line in log_lines:
        print(f"    {line}")

    return TestResult(
        test_name=name,
        is_passed=(return_code == 0 and not timed_out),
        time_taken_sec=int(time.time() - test_start),
        timeout_sec=timeout_sec,
        log_lines=log_lines,
        return_code=return_code,
    )


def run_tests(include_optional=False):
    """Run selected scenarios in ORDER. Returns list of TestResult."""
    pwd = os.path.dirname(os.path.abspath(__file__))
    selected, _ = select_scenarios(include_optional)
    return [
        _run_single_test(os.path.join(pwd, f"{name}.py"), name, timeout)
        for name, timeout, _ in selected
    ]


def _parse_args():
    parser = argparse.ArgumentParser(description="Run foc-devnet scenario tests")
    parser.add_argument(
        "--include-optional",
        action="store_true",
        help="run optional extended scenarios after the core scenarios",
    )
    return parser.parse_args()


def _print_summary(results, elapsed):
    """Print a human-readable summary to stdout."""
    passed = sum(1 for r in results if r.is_passed)
    failed = len(results) - passed
    print(f"\n{'='*50}")
    print(
        f"Scenarios: {len(results)}  Passed: {passed}  Failed: {failed}  ({elapsed}s)"
    )
    for r in results:
        timed_out = r.return_code != 0 and r.time_taken_sec >= r.timeout_sec
        icon = "✅" if r.is_passed else "❌"
        text = "TIMEOUT" if timed_out else ("PASS" if r.is_passed else "FAIL")
        print(f"  {icon} {r.test_name}: {text} ({r.time_taken_sec}s)")


def _print_ci_url():
    """Print CI run/job URL if running in GitHub Actions."""
    run_id = os.environ.get("GITHUB_RUN_ID")
    repo = os.environ.get("GITHUB_REPOSITORY")
    if not (run_id and repo):
        return
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    ci_url = f"{server}/{repo}/actions/runs/{run_id}"
    ci_url_type = "run"
    job_id = os.environ.get("GITHUB_CI_JOB_ID")
    if job_id:
        ci_url = f"{server}/{repo}/actions/runs/{run_id}/job/{job_id}"
        ci_url_type = "job"
    print(f"CI {ci_url_type}: {ci_url}")


if __name__ == "__main__":
    args = _parse_args()
    _, skipped = select_scenarios(args.include_optional)
    selection = "core + optional" if args.include_optional else "core"
    info(f"Scenario selection: {selection}")
    if skipped:
        info(
            "Skipping optional scenarios (use --include-optional): "
            + ", ".join(name for name, _, _ in skipped)
        )
    start = time.time()
    results = run_tests(args.include_optional)
    elapsed = int(time.time() - start)
    _print_summary(results, elapsed)
    print(
        f"Report: {write_report(results=results, selection=selection, skipped_tests=[name for name, _, _ in skipped])}"
    )
    _print_ci_url()
    sys.exit(0 if all(r.is_passed for r in results) else 1)
