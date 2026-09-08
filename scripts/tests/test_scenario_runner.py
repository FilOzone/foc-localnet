"""Unit tests for scenario selection without starting a devnet."""

import sys
import unittest
from unittest.mock import patch

from scenarios.report import _format_skipped_optional_tests
from scenarios.run import ORDER, _parse_args, select_scenarios


class ScenarioSelectionTests(unittest.TestCase):
    def test_core_selection_excludes_optional_scenarios(self):
        selected, skipped = select_scenarios(False)

        self.assertEqual(
            [entry[0] for entry in skipped],
            ["test_bulk_add", "test_termination_controls"],
        )
        self.assertEqual(
            [entry[0] for entry in selected], [entry[0] for entry in ORDER[:-2]]
        )
        self.assertTrue(all(not entry[2] for entry in selected))

    def test_optional_selection_runs_everything_in_order(self):
        selected, skipped = select_scenarios(True)

        self.assertEqual(selected, ORDER)
        self.assertEqual(skipped, [])

    def test_include_optional_cli_flag_selects_every_scenario(self):
        with patch.object(sys, "argv", ["run.py", "--include-optional"]):
            args = _parse_args()

        selected, skipped = select_scenarios(args.include_optional)
        self.assertTrue(args.include_optional)
        self.assertEqual(selected, ORDER)
        self.assertEqual(skipped, [])

    def test_selection_preserves_custom_order_and_report_data(self):
        order = [("core", 1, False), ("extended", 2, True)]

        selected, skipped = select_scenarios(False, order)

        self.assertEqual(selected, [("core", 1, False)])
        self.assertEqual(skipped, [("extended", 2, True)])

    def test_skipped_scenarios_are_rendered_for_the_report(self):
        _, skipped = select_scenarios(False)

        rendered = _format_skipped_optional_tests([entry[0] for entry in skipped])

        self.assertIn("`test_bulk_add`", rendered)
        self.assertIn("`test_termination_controls`", rendered)


if __name__ == "__main__":
    unittest.main()
