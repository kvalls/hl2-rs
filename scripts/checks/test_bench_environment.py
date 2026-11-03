"""Synthetic counters and alignment; no game, host queries or GPU required."""
import json
import plistlib
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace
from bench_environment import align, correlation, known_interference, pair_environment, report, throughput
from bench_environment_macos import Probe, cpu_percent, cpu_seconds, gpu_stats, vm_pages


class EnvironmentTests(unittest.TestCase):
    def test_unavailable_cpu_or_gpu_reading_is_not_a_quiet_baseline(self):
        from bench_environment import quiet_sample
        sample = {"cpu_busy_percent": 0, "gpu_busy_percent": 0, "memory_pressure_level": 1}
        self.assertTrue(quiet_sample(sample))
        for key in ("cpu_busy_percent", "gpu_busy_percent", "memory_pressure_level"):
            self.assertFalse(quiet_sample({**sample, key: None}))
        self.assertFalse(quiet_sample({**sample, "cpu_busy_percent": float("nan")}))

    def test_preflight_distinguishes_known_workloads_from_moderate_utilisation(self):
        self.assertEqual(known_interference([{"cpu_busy_percent": 40, "gpu_busy_percent": 35}]), [])
        self.assertEqual(known_interference([{"other_game_processes": None}, {}]), [])
        self.assertEqual(known_interference([{"other_game_processes": 1},
                                             {"competing_build_processes": 2}]),
                         ["competing_build_processes", "other_game_processes"])

    def test_busy_preflight_saves_evidence_without_launching_a_game(self):
        import check_blur_performance as guard
        baseline = [{"other_game_processes": 1}]
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp)
            with patch.object(guard, "Monitor") as monitor, \
                 patch.object(guard.replay, "OUT", out), \
                 patch.object(guard.replay, "CONFIG", out/"settings.json"), \
                 patch.object(guard.replay, "settings"), \
                 patch.object(guard.replay, "game") as game, \
                 patch("builtins.print"):
                monitor.return_value.__enter__.return_value = SimpleNamespace(
                    baseline=baseline, baseline_history=baseline, metadata={})
                with self.assertRaises(guard.replay.Failure):
                    guard.measure("still", True, 0)
                game.assert_not_called()
            evidence = json.loads((out/"still-advanced-0.preflight.json").read_text())
            self.assertEqual(evidence["known_interference"], ["other_game_processes"])
            self.assertEqual(evidence["baseline"], baseline)

    def test_focused_scenes_are_explicit_and_validate_the_workload(self):
        from check_blur_performance import requested_scenes
        self.assertEqual(requested_scenes("still turn swing"), ("still", "turn", "swing"))
        self.assertEqual(requested_scenes("turn"), ("turn",))
        for value in ("", "still still", "camera", "turn missing"):
            with self.assertRaises(ValueError):
                requested_scenes(value)

    def test_cpu_delta_is_host_normalized_and_handles_wrap(self):
        self.assertIsNone(cpu_percent(None, [1, 2, 3, 4]))
        self.assertEqual(cpu_percent([10, 20, 30, 0], [20, 25, 45, 0]), 50)
        self.assertEqual(cpu_percent([(1 << 32)-5, 0, 0, 0], [5, 0, 10, 0]), 50)
        self.assertIsNone(cpu_percent([0]*4, [0]*4))

    def test_unified_memory_counts_do_not_invent_dedicated_vram(self):
        result = vm_pages('page size of 16384 bytes\nPages free: 3.\nPages inactive: 4.\nPages speculative: 5.\nPages purgeable: 9.\nSwapouts: 2.')
        self.assertEqual(result['memory_available_estimate_bytes'], 12*16384)
        self.assertEqual(result['swapout_bytes_counter'], 2*16384)
        self.assertNotIn('dedicated_vram_available_bytes', result)
        gpu = gpu_stats(plistlib.dumps([{'PerformanceStatistics': {'Device Utilization %': 75,
            'Alloc system memory': 900, 'In use system memory': 123}}]))
        self.assertEqual(gpu['gpu_busy_percent'], 75)
        self.assertEqual(gpu['gpu_driver_allocated_system_bytes'], 900)
        self.assertNotIn('dedicated_vram_available_bytes', gpu)
        self.assertNotIn('gpu_busy_percent', gpu_stats(plistlib.dumps([{'PerformanceStatistics': {}}]*2)))

    def test_alignment_excludes_startup_and_does_not_double_count_boundaries(self):
        samples = [{'unix_seconds': t, 'gpu_busy_percent': 20*t} for t in (0., 1., 2., 3.)]
        windows = [{'start_unix_seconds': 1., 'end_unix_seconds': 2.},
                   {'start_unix_seconds': 2., 'end_unix_seconds': 3.}]
        rows = align(samples, windows)
        self.assertEqual([r['environment']['gpu_busy_percent'] for r in rows], [20., 40.])
        self.assertEqual([r['resource_samples'] for r in rows], [1, 1])

    def test_ratios_are_defined_and_missing_counters_stay_missing(self):
        result = throughput(50, 1000, 1000, {'game_cpu_percent_one_core': 200, 'gpu_busy_percent': 50})
        self.assertEqual(result['megapixels_per_second'], 50)
        self.assertEqual(result['frames_per_game_cpu_second'], 25)
        self.assertEqual(result['megapixels_per_system_gpu_busy_second_proxy'], 100)
        self.assertIsNone(throughput(50, 1, 1, {})['frames_per_game_cpu_second'])
        self.assertIsNone(correlation([(1, 1)]*10)['pearson_r'])
        self.assertIsNone(correlation([(1, 1)])['pearson_r'])
        self.assertAlmostEqual(correlation([(i, i*2) for i in range(6)])['pearson_r'], 1)

    def test_work_per_frame_does_not_invent_energy_from_utilisation(self):
        result = throughput(50, 1000, 1000, {"game_cpu_percent_one_core": 200, "gpu_busy_percent": 50})
        self.assertEqual(result["game_cpu_ms_per_frame"], 40)
        self.assertEqual(result["system_gpu_busy_ms_per_frame_proxy"], 10)
        self.assertIsNone(result["power_watts"])
        self.assertIsNone(result["energy_joules_per_frame"])
        for fps, resources in [(0, {}), (50, {})]:
            unknown = throughput(fps, 1, 1, resources)
            self.assertIsNone(unknown["game_cpu_ms_per_frame"])
            self.assertIsNone(unknown["system_gpu_busy_ms_per_frame_proxy"])

    def test_reports_flag_competition_pressure_and_unknown_telemetry(self):
        samples = [{'unix_seconds': i+0.5, 'sampler_wall_ms': 1, 'gpu_busy_percent': 80,
                    'cpu_busy_percent': 20, 'memory_pressure_level': 2, 'other_game_processes': 1,
                    'swapout_bytes_counter': i*100} for i in range(8)]
        frame = {'mean_fps': 50, 'frame_windows': {'windows': [
            {'start_unix_seconds': i, 'end_unix_seconds': i+1, 'mean_frame_ms': i+20} for i in range(8)]}}
        result = report(samples, [], frame, {'memory_model': 'unified'}, (1024, 768))
        self.assertIn('other_game_processes', result['quality_flags'])
        self.assertIn('memory pressure during measured frames', result['quality_flags'])
        self.assertIn('swapout_bytes_counter increased', result['quality_flags'])
        self.assertEqual(result['resource_sample_count'], 8)
        self.assertFalse(pair_environment(result, result)['comparable'])
        unknown = report([], [], frame, {}, (1024, 768))
        self.assertIn('gpu_busy_percent unavailable or insufficient', unknown['quality_flags'])

    def test_hardware_and_ambient_load_mismatches_are_not_normalized_away(self):
        a = {'quality_flags': [], 'hardware': {'cpu': 'A', 'memory_total_bytes': 16*1024**3},
             'baseline': {'cpu_busy_percent': 10, 'gpu_busy_percent': 10, 'memory_available_estimate_bytes': 8*1024**3}}
        b = {'quality_flags': [], 'hardware': {'cpu': 'B', 'memory_total_bytes': 16*1024**3},
             'baseline': {'cpu_busy_percent': 40, 'gpu_busy_percent': 50, 'memory_available_estimate_bytes': 2*1024**3}}
        result = pair_environment(a, b)
        self.assertFalse(result['comparable'])
        self.assertIn('hardware differs: cpu', result['flags'])
        self.assertIn('baseline available memory differs materially', result['flags'])
        self.assertTrue(any('percentage points' in flag for flag in result['flags']))

    def test_resource_table_keeps_all_scenes_in_one_markdown_table(self):
        from blur_environment_report import markdown
        environment = {'active_mean': {}, 'throughput': {'megapixels_per_second': 1,
                       'frames_per_game_cpu_second': None}, 'active_min_available_memory_bytes': None,
                       'quality_flags': [], 'within_run_associations_with_frame_ms': {}}
        run = {'environment': environment, 'mean_frame_ms': 20, 'p95_frame_ms_upper_bound': 25}
        pair = {'retail': [run], 'advanced': [run], 'frame_time_ratio_advanced_to_retail': 1,
                'environment_pairs': []}
        text = markdown({'still': pair, 'turn': pair})
        table = text.split('## Paired costs')[0]
        self.assertEqual(sum(row.startswith('| still') or row.startswith('| turn') for row in table.splitlines()), 4)
        self.assertNotIn('frame-time ratio', table)
        self.assertIn('unavailable', text)

    def test_baseline_excludes_stale_gpu_primer(self):
        from bench_environment import Monitor, Unavailable
        class FakeProbe(Unavailable):
            def __init__(self):
                self.calls = 0
            def sample(self):
                self.calls += 1
                return {'gpu_busy_percent': 99 if self.calls == 1 else 0, 'cpu_busy_percent': 0}
        with patch('bench_environment.platform.system', return_value='Unsupported'):
            monitor = Monitor('owned-game', interval=0.001)
        monitor.probe = FakeProbe()
        with monitor:
            pass
        self.assertEqual(monitor.primer['gpu_busy_percent'], 99)
        self.assertEqual(len(monitor.baseline), 2)
        self.assertEqual(monitor.baseline[0]['gpu_busy_percent'], 0)

    def test_matched_moderate_load_is_comparable_without_assuming_an_idle_machine(self):
        run = {'quality_flags': [], 'hardware': {'cpu': 'A', 'quiet_baseline_observed': False},
               'baseline': {'cpu_busy_percent': 23, 'gpu_busy_percent': 24}}
        result = pair_environment(run, run)
        self.assertTrue(result['comparable'])
        self.assertFalse(result['both_baselines_quiet'])

    def test_process_cpu_uses_elapsed_time_and_no_process_names_are_retained(self):
        self.assertAlmostEqual(cpu_seconds('1-02:03:04.5'), 93784.5)
        probe = object.__new__(Probe)
        probe.usage = SimpleNamespace(sample=lambda pid: {})
        probe.executable = str(Path('owned-game/hl2-bevy').resolve())
        probe.process, probe.previous_time = ('12', 10.), 1.
        result = probe.processes(f'12 0:11.50 1024 {probe.executable}\n13 0:00.0 42 /tools/rustc\n14 0:00.0 42 /another/hl2-bevy', 2.)
        self.assertEqual(result['game_cpu_percent_one_core'], 150)
        self.assertEqual(result['competing_build_processes'], 1)
        self.assertEqual(result['other_game_processes'], 1)
        self.assertEqual(result['game_rss_bytes'], 1048576)
        self.assertNotIn('process_names', result)


if __name__ == '__main__':
    unittest.main()
