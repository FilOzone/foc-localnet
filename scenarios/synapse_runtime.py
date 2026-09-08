#!/usr/bin/env python3
"""Prepare and run the Synapse E2E consumer runtime."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from scenarios.dependencies import component
from scenarios.helpers import fail, info, ok, run_cmd

TRANSIENT_CHAIN_ERRORS = (
    "refusing explicit call due to state fork at epoch",
    "requested epoch was a null round",
)
UPLOAD_RETRY_DELAYS_SECS = (5, 10, 15, 30)


@dataclass(frozen=True)
class SynapseRuntime:
    work_dir: Path
    source: str
    provenance: str


def _scenario_dir() -> Path:
    return Path(__file__).with_name("synapse-e2e")


def _npm_view(package: str, version: str, *fields: str):
    result = subprocess.run(
        ["npm", "view", f"{package}@{version}", *fields, "--json"],
        text=True,
        capture_output=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"npm metadata lookup failed for {package}@{version}: {result.stderr.strip()}"
        )
    value = json.loads(result.stdout)
    if fields == ("version",):
        return _npm_version(value, package, version)
    return value


def _npm_version(value, package: str, requested: str) -> str:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, list):
        for version in reversed(value):
            if isinstance(version, str) and version:
                return version
    raise RuntimeError(f"npm returned no version for {package}@{requested}")


def _npm_runtime_dependencies(dependency: dict) -> dict[str, str]:
    metadata = _npm_view(
        dependency["package"], dependency["version"], "dependencies", "peerDependencies"
    )
    core_range = metadata.get("dependencies", {}).get("@filoz/synapse-core")
    viem_range = metadata.get("peerDependencies", {}).get("viem") or metadata.get(
        "dependencies", {}
    ).get("viem")
    if not isinstance(core_range, str) or not isinstance(viem_range, str):
        raise RuntimeError(
            f"{dependency['package']}@{dependency['version']} must declare "
            "@filoz/synapse-core and viem"
        )
    return {
        "@filoz/synapse-core": _npm_version(
            _npm_view("@filoz/synapse-core", core_range, "version"),
            "@filoz/synapse-core",
            core_range,
        ),
        "viem": _npm_version(
            _npm_view("viem", viem_range, "version"), "viem", viem_range
        ),
    }


def _write_manifest(work_dir: Path, dependency: dict) -> None:
    runtime_dependencies = dependency.get("runtime_dependencies")
    if not isinstance(runtime_dependencies, dict):
        runtime_dependencies = _npm_runtime_dependencies(dependency)

    dependencies = {
        dependency["package"]: dependency["version"],
        **runtime_dependencies,
    }
    overrides = {
        package: spec["version"]
        for package, spec in dependency.get("overrides", {}).items()
    }
    manifest = {
        "name": "foc-devnet-synapse-e2e",
        "private": True,
        "type": "module",
        "dependencies": dependencies,
    }
    if overrides:
        manifest["overrides"] = overrides
    (work_dir / "package.json").write_text(json.dumps(manifest, indent=2) + "\n")


def _copy_scenarios(work_dir: Path) -> None:
    source = _scenario_dir()
    if not source.is_dir():
        raise RuntimeError(f"Synapse scenario directory not found: {source}")
    shutil.copytree(source, work_dir, dirs_exist_ok=True)


def prepare_synapse_runtime(work_dir: Path) -> SynapseRuntime:
    """Install Synapse into a temporary consumer project."""
    work_dir = work_dir.resolve()
    work_dir.mkdir(parents=True, exist_ok=True)
    dependency = component("synapse-sdk")
    source = dependency.get("source")
    if source not in {"npm", "pkg_pr_new"}:
        raise RuntimeError(f"Unsupported Synapse package source: {source!r}")

    _write_manifest(work_dir, dependency)
    if not run_cmd(
        [
            "npm",
            "install",
            "--omit=dev",
            "--ignore-scripts",
            "--package-lock=false",
        ],
        cwd=str(work_dir),
        label="install Synapse consumer runtime",
    ):
        raise RuntimeError("failed to install Synapse consumer runtime")

    identifier = dependency.get("commit") or dependency["version"]
    runtime = SynapseRuntime(
        work_dir,
        source,
        f"{source}:{dependency['package']}@{identifier}",
    )
    _copy_scenarios(work_dir)
    info(f"Synapse runtime: {runtime.provenance}")
    return runtime


def run_node_script(
    runtime: SynapseRuntime,
    script_name: str,
    label: str,
    args: list[str] | None = None,
    env: dict | None = None,
    timeout: int | None = None,
) -> None:
    """Run a prepared scenario entrypoint with retries for transient chain errors."""
    script = runtime.work_dir / script_name
    if not script.is_file():
        raise RuntimeError(f"Synapse scenario entrypoint not found: {script}")

    cmd = ["node", str(script), *(args or [])]
    process_env = {**os.environ, **(env or {})}

    max_attempts = len(UPLOAD_RETRY_DELAYS_SECS) + 1
    for attempt in range(1, max_attempts + 1):
        result = subprocess.run(
            cmd,
            cwd=str(runtime.work_dir),
            env=process_env,
            text=True,
            capture_output=True,
            timeout=timeout,
        )
        details = "\n".join(
            part for part in (result.stderr.strip(), result.stdout.strip()) if part
        )
        if result.returncode == 0:
            if details:
                info(details)
            ok(label)
            return
        if (
            not any(error in details for error in TRANSIENT_CHAIN_ERRORS)
            or attempt == max_attempts
        ):
            fail(f"{label} (exit={result.returncode}) {details}")

        delay = UPLOAD_RETRY_DELAYS_SECS[attempt - 1]
        info(
            f"{label}: Lotus returned a transient chain RPC error; "
            f"retrying in {delay}s (attempt {attempt}/{max_attempts})"
        )
        time.sleep(delay)
