import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scenarios import report


class ScenarioReportTests(unittest.TestCase):
    def test_report_embeds_status(self):
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "scenario_report.md"
            with patch.object(report, "REPORT_MD", str(report_path)):
                with patch.object(report, "get_version_info", return_value="version"):
                    with patch.object(
                        report, "get_status_info", return_value="status output"
                    ):
                        report.write_report()

            content = report_path.read_text()

        self.assertIn("## Devnet status", content)
        self.assertIn("status output", content)

    @patch("scenarios.report.subprocess.run")
    def test_status_output_includes_stderr(self, run):
        run.return_value = subprocess.CompletedProcess(
            ["./foc-devnet", "status"],
            1,
            stdout="status output\n",
            stderr="status warning\n",
        )

        self.assertEqual(report.get_status_info(), "status output\nstatus warning")
