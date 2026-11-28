"""Ensure all prop populations follow the same camera path and retain their class mix."""
import unittest

from prop_scaling import (CASES, MIX, RESETTLE_END, WAKE, WINDOWS, fixture,
                          summarize_wake, wake_fixture, wake_result)


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

    def test_wake_fits_action_limit_and_starts_window_after_both_batches(self):
        reference = {i: {"origin": [float(i), 0., 36.]} for i in range(100)}
        steps = wake_fixture("battery", 100, reference)
        wakes = [step for step in steps if step.get("actions")]
        self.assertEqual([wake["tick"] for wake in wakes], [WAKE - 1, WAKE])
        self.assertEqual([len(wake["actions"]) for wake in wakes], [64, 36])
        self.assertEqual([action["entity"] for wake in wakes for action in wake["actions"]],
                         list(reference))
        self.assertEqual(wakes[-1]["profile_window"], "start")
        self.assertEqual(steps[-1]["tick"], RESETTLE_END)
        self.assertEqual([s["label"] for s in steps if s.get("profile_window") == "end"],
                         list(WINDOWS.values()))

    def test_wake_result_validates_body_states_and_marks_missing_instrumentation(self):
        spawn = {"entity": 10, "class": "item_battery", "prop": "battery.mdl", "draws": 1,
                 "origin": [4., 3., 8.], "load_ms": 6.}
        reference = {10: {"class": "item_battery", "prop": "battery.mdl", "origin": [4., 3., 36.]}}
        stages = {"sim_physics": {"samples": 100, "mean_ms": 0.25}}

        def sample(asleep, height, **facts):
            body = {"entity": 10, "body": [5., asleep, True], "killed": False,
                    "origin": [4., 3., height], **facts}
            return {"gameplay": {"npc_spawns": [spawn], "entities": [body]}, "cpu_stages": stages}

        samples = {"placed": sample(False, 8.), "finish": sample(True, 0.),
                   "awake-start": sample(False, 36.),
                   "landing": sample(False, 0., body_contact=True), "awake-end": sample(False, 0.),
                   "resettled-end": sample(True, 0.)}
        row = wake_result(samples, "battery", 1, reference)
        self.assertEqual(row["checkpoints"]["awake-start"], {"awake": 1, "contacting": None})
        self.assertEqual(row["checkpoints"]["landing"]["contacting"], 1)
        self.assertIsNone(row["phase_stages"]["awake"]["sim_query_refresh"])
        self.assertEqual(row["load_ms"]["median"], 6.)
        summaries = summarize_wake([row])
        self.assertEqual(summaries["checkpoint_ranges"]["awake-start"]["awake"], [1, 1])
        self.assertIsNone(summaries["spawn_total_ms"]["install_ms"])

        samples["awake-start"] = sample(True, 36.)
        with self.assertRaisesRegex(ValueError, "not all props woke"):
            wake_result(samples, "battery", 1, reference)
