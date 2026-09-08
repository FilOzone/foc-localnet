#!/usr/bin/env python3
"""Exercise optional relayed and direct data-set termination controls."""

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
    with tempfile.TemporaryDirectory(prefix="termination-controls-") as tmp:
        runtime = prepare_synapse_runtime(Path(tmp))
        fixture = runtime.work_dir / "termination-controls.bin"
        write_random_file(fixture, FIXTURE_SIZE, seed=127900)
        assert_eq(
            fixture.stat().st_size, FIXTURE_SIZE, "termination controls fixture created"
        )
        info("Running optional relayed and direct termination checks")
        run_node_script(
            runtime,
            "termination-controls.ts",
            "termination controls scenario",
            args=[str(fixture)],
            env={"NETWORK": "devnet"},
        )


if __name__ == "__main__":
    run()
