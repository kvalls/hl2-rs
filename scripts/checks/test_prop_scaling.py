"""Ensure all prop populations follow the same camera path and retain their class mix."""
import unittest

from prop_scaling import CASES, MIX, fixture


class PropScalingTests(unittest.TestCase):
    def test_only_spawn_commands_change_between_populations(self):
        baseline = fixture("empty", 0)
        for kind, count in CASES:
            steps = fixture(kind, count)
            commands = [step.pop("ui")[0]["command"] for step in steps[1:] if "ui" in step]
            self.assertEqual(steps, baseline)
            self.assertEqual(len(commands), count)
            if kind == "mixed":
                self.assertEqual(set(commands), set(MIX))
            if kind == "battery":
                self.assertEqual(set(commands), {MIX[0]})
