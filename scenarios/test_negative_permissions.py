#!/usr/bin/env python3
"""Verify rejected service-provider requests cannot mutate a live data set."""

import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from scenarios.helpers import assert_eq, assert_ok, info, write_random_file
from scenarios.synapse_runtime import prepare_synapse_runtime, run_node_script

FIXTURE_SIZE = 4 * 1024


def run():
    assert_ok("command -v node", "node is installed")
    with tempfile.TemporaryDirectory(prefix="negative-permissions-") as tmp:
        runtime = prepare_synapse_runtime(Path(tmp))
        fixture = runtime.work_dir / "negative-permissions.bin"
        write_random_file(fixture, FIXTURE_SIZE, seed=127)
        assert_eq(
            fixture.stat().st_size, FIXTURE_SIZE, "negative permissions fixture created"
        )
        info(
            "Running permission and malformed-request checks against an isolated data set"
        )
        run_node_script(
            runtime,
            "negative-permissions.ts",
            "negative permissions scenario",
            args=[str(fixture)],
            env={"NETWORK": "devnet"},
        )


if __name__ == "__main__":
    run()
