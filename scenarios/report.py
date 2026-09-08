#!/usr/bin/env python3
"""Report generation and version info for scenario test results."""

from __future__ import annotations

import datetime
import os
import subprocess
from dataclasses import dataclass
from string import Template

from scenarios.dependencies import format_markdown_table

REPORT_MD = os.environ.get(
    "REPORT_FILE", os.path.expanduser("~/.foc-devnet/state/latest/scenario_report.md")
)


@dataclass
class TestResult:
    test_name: str
    is_passed: bool
    time_taken_sec: int
    timeout_sec: int
    log_lines: list[str]
    return_code: int


def get_version_info():
    """Capture output of `foc-devnet version` for inclusion in reports."""
    for binary in ["./foc-devnet", "foc-devnet"]:
        try:
            result = subprocess.run(
                [binary, "version", "--notty"],
                capture_output=True,
                text=True,
                timeout=10,
            )
            if result.returncode == 0:
                return result.stdout.strip()
        except (FileNotFoundError, subprocess.TimeoutExpired):
            continue
    return "foc-devnet version: not available"


_REPORT_TEMPLATE = Template("""
# Scenarios Tests 

| Description | Data                                                                |
|-------------| ------------------------------------------------------------------- |
| Type        | **$run_type**                                                       |
| Date        | $date                                                               |
| Status      | PASS ✅:**$pass_count**, FAIL 🟥:**$fail_count**, Total:$total_count |
| CI run      | $ci_run_link                                                        |

## Versions info
$version_info

## Resolved dependencies
$dependency_table

## Scenario selection
**$scenario_selection**

$skipped_optional_tests

## Tests summary
$test_summary
""")


def _build_ci_run_link():
    """Build the CI run markdown link from environment variables, or '-' for local."""
    github_run_id = os.environ.get("GITHUB_RUN_ID")
    github_repo = os.environ.get("GITHUB_REPOSITORY")
    if github_run_id and github_repo:
        github_server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
        ci_url = f"{github_server}/{github_repo}/actions/runs/{github_run_id}"
        return f"[{ci_url}]({ci_url})"
    return "-"


def _build_test_summary(results: list[TestResult]) -> str:
    """Build collapsible markdown sections for each test result."""
    parts = []
    for r in results:
        timed_out = r.return_code != 0 and r.time_taken_sec >= r.timeout_sec
        icon = "✅" if r.is_passed else "❌"
        status = (
            f"TIMEOUT ({r.time_taken_sec}s)"
            if timed_out
            else f"{'PASS' if r.is_passed else 'FAIL'} ({r.time_taken_sec}s)"
        )
        parts.append(
            f"<details>\n<summary>{icon} <b>{r.test_name}</b> - {status}</summary>\n\n"
            f"```\n{chr(10).join(r.log_lines)}\n```\n</details>"
        )
    return "\n\n".join(parts)


def _format_skipped_optional_tests(skipped_tests: list[str]) -> str:
    if not skipped_tests:
        return "No optional scenarios skipped."
    return "Skipped optional scenarios:\n" + "\n".join(
        f"- `{test_name}`" for test_name in skipped_tests
    )


def write_report(
    results: list[TestResult] | None = None,
    elapsed: int = 0,
    selection: str = "core",
    skipped_tests: list[str] | None = None,
):
    """Write a markdown report to REPORT_MD. Returns path written."""
    if results is None:
        results = []
    if skipped_tests is None:
        skipped_tests = []
    total = len(results)
    passed = sum(1 for r in results if r.is_passed)
    content = _REPORT_TEMPLATE.substitute(
        run_type=os.environ.get("SCENARIO_RUN_TYPE", "local"),
        date=datetime.datetime.now(datetime.UTC).strftime("%d-%B-%Y %H:%M:%S GMT +0"),
        pass_count=passed,
        fail_count=total - passed,
        total_count=total,
        ci_run_link=_build_ci_run_link(),
        version_info=f"```\n{get_version_info()}\n```",
        dependency_table=format_markdown_table(),
        scenario_selection=selection,
        skipped_optional_tests=_format_skipped_optional_tests(skipped_tests),
        test_summary=_build_test_summary(results),
    )
    with open(REPORT_MD, "w") as fh:
        fh.write(content)
    return REPORT_MD
