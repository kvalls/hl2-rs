"""Controls for the O32 scaling workload."""
import unittest

from crowd_scaling import COUNTS, fixture, sight_share


class CrowdScalingTests(unittest.TestCase):
    def test_only_spawn_commands_change_between_populations(self):
        reference = fixture(0)
        for count in COUNTS:
            steps = fixture(count)
            commands = [step.pop("ui") for step in steps[1:] if "ui" in step]
            self.assertEqual(len(commands), count)
            self.assertEqual(len({c[0]["command"] for c in commands}), count)
            self.assertEqual(steps, reference)
            self.assertEqual(len({tuple(step["eye"]) for step in steps[1:101]}), 100)

    def test_nested_sight_stacks_count_once_within_fixed_step(self):
        graph = """Call graph:
    10 Thread_main
    + 9 fixed_step
    +   5 PhysicsSight::visible
    +     5 PhysicsSight::first_solid
    +   4 work
    + 1 PhysicsSight::first_solid
Total number in stack
"""
        self.assertEqual(sight_share(graph), {"fixed_samples": 9, "sight_samples": 5})
