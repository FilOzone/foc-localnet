#!/usr/bin/env python3
"""Exercise the optional many-piece and lockup-replenishment path."""

import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from scenarios.helpers import assert_eq, assert_ok, info, write_random_file
from scenarios.synapse_runtime import prepare_synapse_runtime, run_node_script

BOOTSTRAP_SIZE = 64 * 1024


def run():
    assert_ok("command -v node", "node is installed")
    with tempfile.TemporaryDirectory(prefix="bulk-add-") as tmp:
        runtime = prepare_synapse_runtime(Path(tmp))
        fixture = runtime.work_dir / "bulk-bootstrap.bin"
        write_random_file(fixture, BOOTSTRAP_SIZE, seed=12740)
        assert_eq(
            fixture.stat().st_size, BOOTSTRAP_SIZE, "bulk-add bootstrap fixture created"
        )
        info("Running optional 40-piece add and lockup-replenishment checks")
        run_node_script(
            runtime,
            "bulk-add.ts",
            "bulk add scenario",
            args=[str(fixture)],
            env={"NETWORK": "devnet"},
        )


if __name__ == "__main__":
    run()
