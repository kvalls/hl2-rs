"""Cross-platform unit tests for HL2-RS `/hl2-performance-trace` scripts and skill docs."""

import argparse
import contextlib
import hashlib
import io
import json
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
SKILL_DIR = REPO / ".agents" / "skills" / "hl2-performance-trace"
SCRIPTS_DIR = SKILL_DIR / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import capture  # noqa: E402
import metal_gpu  # noqa: E402
import resource_sampler  # noqa: E402
import sample_summary  # noqa: E402
import trace_summary  # noqa: E402
import trace_waste  # noqa: E402


def _write_events(path: Path, events: list[dict]) -> Path:
    lines = ["[\n"] + [json.dumps(ev) + (",\n" if i + 1 < len(events) else "\n") for i, ev in enumerate(events)] + ["]\n"]
    path.write_text("".join(lines), encoding="utf-8")
    return path


def _synthetic_trace_events() -> list[dict]:
    return [
        {"ph": "M", "name": "thread_name", "tid": 1, "args": {"name": "main"}},
        # Frame 0: fast (5.0 ms), contains one true duplicate actor_pose across two systems.
        {"ph": "B", "name": "main app", "tid": 1, "ts": 1_000},
        {"ph": "B", "name": "state_frame: allocations=100 bytes=4096", "tid": 1, "ts": 1_100},
        {"ph": "E", "tid": 1, "ts": 1_100},
        {"ph": "B", "name": "state_npc: id=7 class=npc_metropolice schedule=Idle task=Wait distance=120.0", "tid": 1, "ts": 1_200},
        {"ph": "E", "tid": 1, "ts": 1_200},
        {"ph": "B", "name": 'system: name="hl2_bevy::rendering::present_entities"', "tid": 1, "ts": 1_300},
        {"ph": "B", "name": "actor_pose: id=7 partial=false", "tid": 1, "ts": 1_350},
        {"ph": "E", "tid": 1, "ts": 2_050},  # 0.7 ms
        {"ph": "E", "tid": 1, "ts": 2_200},
        {"ph": "B", "name": 'system: name="hl2_bevy::eyes::present"', "tid": 1, "ts": 2_300},
        {"ph": "B", "name": "actor_pose: id=7 partial=false", "tid": 1, "ts": 2_350},
        {"ph": "E", "tid": 1, "ts": 2_850},  # 0.5 ms duplicate
        {"ph": "E", "tid": 1, "ts": 3_000},
        {"ph": "E", "tid": 1, "ts": 6_000},  # main app frame 0 ends at 6000 us (5 ms)
        # Frame 1: slow (18.0 ms), contains two separate fixed-tick runs of one AI system (not a duplicate).
        {"ph": "B", "name": "main app", "tid": 1, "ts": 11_000},  # 10 ms start-to-start (> 8.33 ms)
        {"ph": "B", "name": "state_frame: allocations=200 bytes=8192", "tid": 1, "ts": 11_100},
        {"ph": "E", "tid": 1, "ts": 11_100},
        {"ph": "B", "name": "state_player: origin=\"0 0 64\" speed=220", "tid": 1, "ts": 11_200},
        {"ph": "E", "tid": 1, "ts": 11_200},
        {"ph": "B", "name": 'system: name="hl2_bevy::gameplay::ai_system"', "tid": 1, "ts": 11_500},
        {"ph": "B", "name": "npc_ai: actors=1", "tid": 1, "ts": 11_600},
        {"ph": "B", "name": "npc: id=7", "tid": 1, "ts": 11_700},
        {"ph": "E", "tid": 1, "ts": 14_700},  # 3.0 ms
        {"ph": "E", "tid": 1, "ts": 14_800},
        {"ph": "E", "tid": 1, "ts": 15_000},
        {"ph": "B", "name": 'system: name="hl2_bevy::gameplay::ai_system"', "tid": 1, "ts": 16_000},
        {"ph": "B", "name": "npc_ai: actors=1", "tid": 1, "ts": 16_100},
        {"ph": "B", "name": "npc: id=7", "tid": 1, "ts": 16_200},
        {"ph": "E", "tid": 1, "ts": 18_200},  # 2.0 ms
        {"ph": "E", "tid": 1, "ts": 18_300},
        {"ph": "E", "tid": 1, "ts": 18_500},
        {"ph": "X", "name": "hl2_bevy::render_stage::slow_pass", "tid": 2, "ts": 19_000, "dur": 4_000},
        {"ph": "E", "tid": 1, "ts": 29_000},  # main app frame 1 = 18 ms
    ]


class TestTraceSummary(unittest.TestCase):
    def test_analyze_and_format_trace(self):
        with tempfile.TemporaryDirectory() as tmp:
            trace = _write_events(Path(tmp) / "trace.json", _synthetic_trace_events())
            short_data = trace_summary.analyze_trace(trace, full_paths=False, spikes=1)
            self.assertEqual(short_data["frames"], 2)
            self.assertEqual(short_data["threads"], 1)
            self.assertAlmostEqual(short_data["frame_ms"]["max"], 18.0, places=2)
            names = {row["span"] for row in short_data["rows"]}
            self.assertIn("slow_pass", names)
            self.assertNotIn("hl2_bevy::render_stage::slow_pass", names)
            self.assertEqual(len(short_data["slow_frames"]), 1)
            self.assertEqual(short_data["slow_frames"][0]["frame"], 1)
            self.assertTrue(any(item["span"] == "slow_pass" for item in short_data["slow_frames"][0]["spans"]))

            full_data = trace_summary.analyze_trace(trace, full_paths=True, sort_key="max", match="slow_pass")
            self.assertEqual([row["span"] for row in full_data["rows"]], ["hl2_bevy::render_stage::slow_pass"])

            out_json = Path(tmp) / "out.json"
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = trace_summary.main(["--trace", str(trace), "--json-out", str(out_json)])
            self.assertEqual(rc, 0)
            self.assertTrue(out_json.is_file())
            self.assertIn("frames ('main app'): 2", buf.getvalue())


class TestTraceWaste(unittest.TestCase):
    def test_separates_true_duplicates_from_multi_tick_and_joins_npc_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            trace = _write_events(Path(tmp) / "trace.json", _synthetic_trace_events())
            rep = trace_waste.analyze_waste(trace, slow_n=2, near=500.0)
            self.assertEqual(rep["frames"], 2)
            self.assertEqual(rep["pacing"]["intervals"], 1)
            self.assertEqual(rep["pacing"]["over_8_33"], 1)
            self.assertEqual(rep["allocations"]["mean"], 150.0)
            self.assertEqual(rep["allocations"]["max"], 200)

            # `actor_pose` called by both `present_entities` and `eyes::present` in Frame 0 is a real duplicate.
            self.assertIn("actor_pose", rep["repeats"])
            self.assertEqual(rep["repeats"]["actor_pose"]["extra_calls"], 1)
            self.assertAlmostEqual(rep["repeats"]["actor_pose"]["extra_ms"], 0.5, places=3)
            self.assertIn("present + present_entities", rep["repeats"]["actor_pose"]["callers"][0][0])

            # `npc_ai` called twice across distinct runs of the same system is NOT flagged as duplicate waste.
            self.assertNotIn("npc_ai", rep["repeats"])
            self.assertEqual(rep["per_tick_repeats"]["npc_ai"], 1)

            # Frame 1 inherited `state_npc` (`npc_metropolice`, `Idle`, `Wait`) from Frame 0.
            self.assertIn("npc_metropolice", rep["npc_cost"]["class"])
            self.assertEqual(rep["npc_cost"]["class"]["npc_metropolice"]["calls"], 2)
            self.assertEqual(rep["slow_frames"][0]["nearby_npcs"]["count"], 1)

            text = trace_waste.format_report(rep)
            self.assertIn("1. Repeated identical work in one frame", text)
            self.assertIn("not duplicates, one per run of the same system", text)


class TestResourceSampler(unittest.TestCase):
    def test_macos_parsers(self):
        ps_text = "  %CPU STAT\n 12.4 R\n 89.2 S\n  3.1 R\n"
        self.assertEqual(resource_sampler.parse_macos_ps_threads(ps_text), [89.2, 12.4, 3.1])

        ioreg_text = (
            '    "PerformanceStatistics" = {"Device Utilization %"=78,'
            '"Renderer Utilization %"=72,"Tiler Utilization %"=41,'
            '"In use system memory"=1073741824,"Alloc system memory"=2147483648}\n'
        )
        gpu = resource_sampler.parse_ioreg_gpu(ioreg_text)
        self.assertEqual(gpu["device_pct"], 78.0)
        self.assertEqual(gpu["in_use_mb"], 1024.0)

        swap_mb, free_mb = resource_sampler.parse_macos_memory(
            "total = 2048.00M  used = 512.50M  free = 1535.50M",
            "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free: 64.\nPages speculative: 64.\n",
        )
        self.assertEqual(swap_mb, 512.5)
        self.assertEqual(free_mb, 2.0)
        self.assertEqual(resource_sampler.parse_macos_thermal("CPU_Speed_Limit = 100\n"), 100)

    def test_summarize_samples(self):
        with tempfile.TemporaryDirectory() as tmp:
            jsonl = Path(tmp) / "resources.jsonl"
            rows = [
                {"t": 100.0, "cpu_pct": 120.0, "rss_mb": 2000.0, "threads": [70.0], "gpu": {"device_pct": 40.0, "in_use_mb": 500.0}, "swap_used_mb": 100.0, "free_mb": 800.0, "thermal": 100},
                {"t": 102.0, "cpu_pct": 180.0, "rss_mb": 2200.0, "threads": [90.0], "gpu": {"device_pct": 80.0, "in_use_mb": 700.0}, "swap_used_mb": 140.0, "free_mb": 600.0, "thermal": 95},
            ]
            jsonl.write_text("\n".join(json.dumps(r) for r in rows) + "\n", encoding="utf-8")
            summ = resource_sampler.summarize_samples(jsonl)
            self.assertEqual(summ["samples"], 2)
            self.assertEqual(summ["duration_s"], 2.0)
            self.assertEqual(summ["gpu_device_pct"]["max"], 80.0)
            self.assertEqual(summ["swap_delta_mb"], 40.0)
            self.assertEqual(summ["thermal_min"], 95.0)
            self.assertIn("delta=+40.0 MB", resource_sampler.format_summary(summ))


class TestMetalGpu(unittest.TestCase):
    def test_analyze_gpu_intervals_with_xml_refs(self):
        xml = """<trace-query-result>
  <node><row>
    <start>10000000</start><duration>4000000</duration><channel fmt="Fragment"/><frame>1</frame><lat>0</lat><depth>1</depth>
    <label id="lbl1"><string>opaque_3d (Render)</string></label><state/><conn/><col/><proc id="p1" fmt="hl2-bevy (123)"/>
  </row></node>
  <node><row>
    <start>12000000</start><duration>5000000</duration><channel fmt="Compute"/><frame>1</frame><lat>0</lat><depth>1</depth>
    <label ref="lbl1"/><state/><conn/><col/><proc ref="p1"/>
  </row></node>
  <node><row>
    <start>18000000</start><duration>2000000</duration><channel fmt="Fragment"/><frame>1</frame><lat>0</lat><depth>1</depth>
    <label><string id="s2">upscaling</string></label><state/><conn/><col/><proc ref="p1"/>
  </row></node>
  <node><row>
    <start>11000000</start><duration>9000000</duration><channel fmt="Fragment"/><frame>1</frame><lat>0</lat><depth>1</depth>
    <label><string>compositor</string></label><state/><conn/><col/><proc fmt="WindowServer"/>
  </row></node>
</trace-query-result>
"""
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "gpu.xml"
            p.write_text(xml, encoding="utf-8")
            data = metal_gpu.analyze_gpu_intervals(p)
            self.assertEqual(data["intervals"], 3)
            self.assertEqual(data["frames"], 1)
            # Union of [10, 14] ms, [12, 17] ms, [18, 20] ms => [10, 17] (7 ms) + [18, 20] (2 ms) = 9.0 ms (> 8.33 ms).
            self.assertAlmostEqual(data["busy_stats"]["mean"], 9.0, places=3)
            self.assertEqual(data["busy_stats"]["over_8_33"], 1)
            self.assertIn("GPU busy per frame", metal_gpu.format_gpu_intervals(data))

    def test_analyze_gpu_intervals_includes_depth_zero(self):
        xml = """<trace-query-result>
  <node><row>
    <start>1000000</start><duration>500000</duration><channel fmt="Fragment"/><frame>1</frame><lat>0</lat><depth>0</depth>
    <label><string>early prepass:early prepass</string></label><state/><conn/><col/><proc id="p1" fmt="hl2-bevy (1)"/>
  </row></node>
  <node><row>
    <start>2000000</start><duration>200000</duration><channel fmt="Fragment"/><frame>1</frame><lat>0</lat><depth>0</depth>
    <label><string>upscaling:upscaling</string></label><state/><conn/><col/><proc ref="p1"/>
  </row></node>
</trace-query-result>
"""
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "gpu0.xml"
            p.write_text(xml, encoding="utf-8")
            data = metal_gpu.analyze_gpu_intervals(p)
            self.assertEqual(data["frames"], 1)
            by_label = {item["label"]: item for item in data["passes"]}
            self.assertIn("early prepass:early prepass", by_label)
            self.assertAlmostEqual(by_label["early prepass:early prepass"]["ms_per_frame"], 0.5, places=3)
            self.assertAlmostEqual(data["busy_stats"]["mean"], 0.7, places=3)

    def test_analyze_display_pacing(self):
        row = lambda dur_ns, fmt: (
            "<row>" + "<c/>" + f"<dur>{dur_ns}</dur>" + "<c/>" * 11 + f'<res fmt="{fmt}"/>' + "</row>"
        )
        xml = "<trace-query-result>" + row(8_333_333, "3456x2168") + row(16_666_666, "3456x2168") + "</trace-query-result>"
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "disp.xml"
            p.write_text(xml, encoding="utf-8")
            data = metal_gpu.analyze_display_pacing(p, refresh_hz=120.0)
            self.assertEqual(data["total_frames"], 2)
            self.assertEqual(data["buckets"], {"1": 1, "2": 1})
            self.assertEqual(data["repeated_refreshes"], 1)
            self.assertAlmostEqual(data["effective_fps"], 80.0, places=0)

    def test_display_pacing_matches_process_and_pid_before_choosing_resolution(self):
        def row(duration, label, resolution):
            return (
                f"<row><c/><duration>{duration}</duration>"
                + "<c/>" * 7 + label + "<c/>" * 3 + resolution + "</row>"
            )

        target = '<narrative id="4" fmt="Surface 11 (hl2-bevy (42):Frame 1:Surface 93 via WindowServer)"/>'
        other = '<narrative fmt="Surface 9 (hl2-bevy (420):Frame 3:Surface 44 via WindowServer)"/>'
        desktop = '<narrative fmt="Surface 8"/>'
        resolution = '<resolution id="5" fmt="(3456,2234)"/>'
        xml = "<trace-query-result>" + "".join((
            row(8_333_333, target, resolution),
            row(16_666_666, '<narrative ref="4"/>', '<resolution ref="5"/>'),
            row(33_333_333, other, '<resolution ref="5"/>'),
            row(33_333_333, desktop, '<resolution ref="5"/>'),
            *(row(25_000_000, desktop, '<resolution fmt="(3840,2160)"/>') for _ in range(5)),
        )) + "</trace-query-result>"
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "displayed_surfaces.xml"
            p.write_text(xml, encoding="utf-8")
            data = metal_gpu.analyze_display_pacing(p, process="hl2-bevy (42)")
            self.assertEqual(data["surface"], "(3456,2234)")
            self.assertEqual(data["process"], "hl2-bevy (42)")
            self.assertEqual(data["buckets"], {"1": 1, "2": 1})
            self.assertEqual(metal_gpu.analyze_display_pacing(p, process="hl2-bevy")["total_frames"], 3)
            with self.assertRaises(metal_gpu.NoAttributedFrames):
                metal_gpu.analyze_display_pacing(p, process="hl2-bevy (4)")


class TestSampleSummary(unittest.TestCase):
    def test_inclusive_dedup_recursion_and_callers(self):
        sample_txt = """Analysis of sampling hl2-bevy (pid 1):
Call graph:
    100 Thread_1001 DispatchQueue_1: com.apple.main-thread
    + 100 main_loop  (in hl2-bevy) + 12 [0x1]
    +   80 recurse_fn  (in hl2-bevy) + 4 [0x2]
    +     30 recurse_fn  (in hl2-bevy) + 16 [0x2]
    +     20 leaf_work  (in hl2-bevy) + 8 [0x3]
    50 Thread_1002 Worker
    + 50 bg_job  (in hl2-bevy) + 1 [0x4]
Total number in stack (recursive counted multiple, when >=5):
"""
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "sample.txt"
            p.write_text(sample_txt, encoding="utf-8")
            inc = sample_summary.analyze_sample(p, thread_filter="main-thread")
            counts = {r["symbol"]: r["count"] for r in inc["rows"]}
            # Recursive inner call does not double-count `recurse_fn`.
            self.assertEqual(counts["recurse_fn"], 80)
            self.assertEqual(counts["leaf_work"], 20)
            self.assertNotIn("bg_job", counts)

            callers = sample_summary.analyze_sample(p, callers_pattern="leaf_work")
            self.assertEqual(callers["rows"], [{"count": 20, "symbol": "recurse_fn"}])


class TestCaptureOrchestrator(unittest.TestCase):
    def test_game_report_preserves_percentile_bounds_and_legacy_exact_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = Path(tmp) / "report.json"
            report.write_text(json.dumps({"presentation": {"cpu_stages": {
                "legacy": {"samples": 80, "p95_ms": 2.4, "max_ms": 3.5},
                "long": {
                    "samples": 6170, "p95_ms": 5.0625, "p99_ms": 8.5,
                    "percentiles_exact": False, "max_ms": 19.5,
                },
            }}}), encoding="utf-8")
            summary = capture.summarize_game_report(report)
            long, legacy = summary["cpu_stages"]
            self.assertEqual((long["samples"], long["p99_ms"], long["percentiles_exact"]),
                             (6170, 8.5, False))
            self.assertIsNone(legacy["p99_ms"])
            self.assertTrue(legacy["percentiles_exact"])
            lines = capture.format_game_report(summary).splitlines()
            self.assertEqual(lines[2].split()[-3:], ["<=5.063", "<=8.500", "19.50"])
            self.assertEqual(lines[3].split()[-3:], ["2.400", "-", "3.50"])

    def test_mode_validation_and_build_commands(self):
        ok, _ = capture.build_matches_mode({"plugins": "debug-tools,advanced-options"}, "stages")
        self.assertTrue(ok)
        ok, msg = capture.build_matches_mode({"plugins": "advanced-options,debug-tools"}, "trace")
        self.assertFalse(ok)
        self.assertIn("capture.py build --mode trace", msg)

        ok, _ = capture.build_matches_mode(
            {"plugins": "advanced-options,debug-tools,trace-alloc,trace-chrome"},
            "trace-alloc",
        )
        self.assertTrue(ok)
        ok, _ = capture.build_matches_mode(
            {"plugins": "advanced-options,debug-tools,trace-alloc,trace-chrome"},
            "trace",
        )
        self.assertFalse(ok)

        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            win_cmd = capture.build_script_command(repo, mode="trace", no_clean=True, is_windows=True)
            self.assertIn("-NoClean", win_cmd)
            self.assertIn("advanced-options,debug-tools,trace-chrome", win_cmd)

            posix_cmd = capture.build_script_command(repo, mode="trace-alloc", profile="dist", no_clean=False, is_windows=False)
            self.assertIn("--dist", posix_cmd)
            self.assertNotIn("--no-clean", posix_cmd)
            self.assertIn("advanced-options,debug-tools,trace-chrome,trace-alloc", posix_cmd)

    def test_analyze_capture_dir_and_delete_raw_safety(self):
        with tempfile.TemporaryDirectory() as tmp:
            cap = Path(tmp) / "cap"
            cap.mkdir()
            _write_events(cap / "trace.json", _synthetic_trace_events())
            (cap / "report.json").write_text(
                json.dumps({
                    "render": {
                        "frames": 120,
                        "adapter": "Mock GPU",
                        "backend": "Metal",
                        "presentation": {
                            "cpu_stages": {
                                "render_views": {"calls": 100, "samples": 80, "mean_ms": 1.2, "p95_ms": 2.4, "max_ms": 3.5}
                            },
                            "hud": {
                                "benchmark": {
                                    "measured_frames": 80,
                                    "mean_frame_ms": 6.5,
                                    "mean_fps": 153.8,
                                    "p95_frame_ms_upper_bound": 8.0,
                                    "p99_frame_ms_upper_bound": 9.0,
                                    "max_frame_ms": 10.0,
                                }
                            },
                        },
                    }
                }),
                encoding="utf-8",
            )
            (cap / "resources.jsonl").write_text(
                json.dumps({"t": 1.0, "cpu_pct": 50.0, "rss_mb": 1000.0, "threads": [40.0], "gpu": None, "swap_used_mb": 10.0, "free_mb": 500.0, "thermal": None}) + "\n",
                encoding="utf-8",
            )
            gpu_trace = cap / "gpu.trace"
            gpu_trace.mkdir()
            (gpu_trace / "dummy").write_text("x", encoding="utf-8")

            # Simulated export error must preserve both raw trace.json and gpu.trace even with delete_raw=True.
            with contextlib.redirect_stdout(io.StringIO()):
                rc_err = capture.analyze_capture_dir(cap, delete_raw=True, metal_exporter=lambda _d: ["simulated export fail"])
            self.assertEqual(rc_err, 1)
            self.assertTrue((cap / "trace.json").exists())
            self.assertTrue(gpu_trace.exists())

            # Without gpu.trace errors and with delete_raw=True, trace.json is cleaned once summaries succeed.
            gpu_trace.joinpath("dummy").unlink()
            gpu_trace.rmdir()
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc_ok = capture.analyze_capture_dir(cap, delete_raw=True)
            self.assertEqual(rc_ok, 0)
            self.assertFalse((cap / "trace.json").exists())
            for expected in (
                "report_summary.txt",
                "report_summary.json",
                "resources_summary.txt",
                "trace_summary.txt",
                "waste_summary.txt",
                "summary.txt",
            ):
                self.assertTrue((cap / expected).is_file(), expected)

    def test_desktop_only_export_does_not_publish_pacing_or_prune_raw(self):
        with tempfile.TemporaryDirectory() as tmp:
            cap = Path(tmp)
            (cap / "gpu.trace").mkdir()
            (cap / "displayed_surfaces.xml").write_text(
                "<trace-query-result><row><c/><duration>8333333</duration>"
                + "<c/>" * 7 + '<narrative fmt="Surface 8"/>' + "<c/>" * 3
                + '<resolution fmt="(3456,2234)"/></row></trace-query-result>',
                encoding="utf-8",
            )
            for suffix in ("txt", "json"):
                (cap / f"display_pacing.{suffix}").write_text("stale", encoding="utf-8")

            with contextlib.redirect_stdout(io.StringIO()) as output:
                rc = capture.analyze_capture_dir(cap, delete_raw=True, metal_exporter=lambda _d: [])
            self.assertEqual(rc, 0)
            self.assertIn("Pacing unavailable: no displayed surfaces attributed to", output.getvalue())
            self.assertTrue((cap / "gpu.trace").is_dir())
            for suffix in ("txt", "json"):
                self.assertFalse((cap / f"display_pacing.{suffix}").exists())

    def test_cleanup_requires_summaries_unless_forced(self):
        with tempfile.TemporaryDirectory() as tmp:
            cap = Path(tmp)
            raw_trace = cap / "trace.json"
            raw_trace.write_text("[]", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
                capture.cmd_cleanup(argparse.Namespace(dir=cap, force=False))
            self.assertTrue(raw_trace.exists())

            with contextlib.redirect_stdout(io.StringIO()):
                capture.cmd_cleanup(argparse.Namespace(dir=cap, force=True))
            self.assertFalse(raw_trace.exists())

    def test_compute_trace_max_mb(self):
        self.assertEqual(capture.compute_trace_max_mb(90.0), (20480, None))
        self.assertEqual(capture.compute_trace_max_mb(18.0), (8192, None))
        cap_mb, err = capture.compute_trace_max_mb(8.5)
        self.assertIsNone(cap_mb)
        self.assertIsNotNone(err)
        self.assertIn("minimum", err)
        self.assertEqual(capture.compute_trace_max_mb(50.0, explicit_env="4096"), (None, None))

    def test_wait_for_gameplay_start(self):
        with tempfile.TemporaryDirectory() as tmp:
            game_log = Path(tmp) / "game.log"
            game_log.write_text("Using Half-Life 2\n", encoding="utf-8")

            polls = 0

            def alive_after_two(_pid: int) -> bool:
                nonlocal polls
                polls += 1
                if polls == 2:
                    with game_log.open("a", encoding="utf-8") as handle:
                        handle.write(f"2026-11-22 INFO {capture.GAMEPLAY_STARTED_MARKER}\n")
                return True

            self.assertTrue(
                capture.wait_for_gameplay_start(
                    game_log,
                    pid=42,
                    poll_interval=0.0,
                    is_alive=alive_after_two,
                )
            )
            self.assertGreaterEqual(polls, 2)

            empty_log = Path(tmp) / "missing.log"
            self.assertFalse(
                capture.wait_for_gameplay_start(
                    empty_log,
                    pid=99,
                    poll_interval=0.0,
                    is_alive=lambda _pid: False,
                )
            )

    def test_lookup_backlog_item_and_status_hash(self):
        item = capture.lookup_backlog_item(REPO, "o2")
        self.assertIsNotNone(item)
        self.assertEqual(item["id"], "O2")
        self.assertIn("Breen-screen", item["measure"])
        self.assertIsNone(capture.lookup_backlog_item(REPO, "O9999"))

        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            missing = capture.collect_status(repo, include_idle_sample=False)
            self.assertFalse(missing["executable_exists"])
            self.assertIsNone(missing["executable_sha256"])

            exe = capture.session.executable_path(repo)
            exe.parent.mkdir(parents=True)
            exe.write_bytes(b"fake-exe")
            (repo / "bin" / "build-bevy-info.json").write_text(
                json.dumps({"profile": "release", "plugins": "advanced-options,debug-tools"}),
                encoding="utf-8",
            )
            ready = capture.collect_status(repo, include_idle_sample=False)
            self.assertTrue(ready["executable_exists"])
            self.assertEqual(ready["executable_sha256"], hashlib.sha256(b"fake-exe").hexdigest())

    def test_headless_stamps_sha256_args_and_backlog_item(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            exe = capture.session.executable_path(repo)
            exe.parent.mkdir(parents=True)
            exe.write_bytes(b"headless-binary")
            (repo / "bin" / "build-bevy-info.json").write_text(
                json.dumps({"profile": "release", "plugins": "advanced-options,debug-tools"}),
                encoding="utf-8",
            )
            docs = repo / "docs"
            docs.mkdir()
            (docs / "optimization-backlog.md").write_text(
                "| ID | Area | Status | Finding | Measure | Next |\n"
                "|---|---|---|---|---|---|\n"
                "| O2 | GPU | open | Five cameras | Static spawn: `render_graph` | Split passes |\n",
                encoding="utf-8",
            )
            out_dir = repo / "out"
            args = argparse.Namespace(
                mode="stages",
                map="d1_trainstation_02",
                frames=10,
                repo=repo,
                out=out_dir,
                name=None,
                item="o2",
                analyze=False,
                delete_raw=False,
                allow_mismatch=False,
                trailing=["--", "--uncapped"],
            )
            idle = {"gpu": {"device_pct": 37.0}}
            with (
                mock.patch.object(capture, "_git_head", return_value="deadbeef"),
                mock.patch.object(
                    capture.subprocess,
                    "run",
                    return_value=capture.subprocess.CompletedProcess(args=[], returncode=0),
                ) as mocked_run,
                mock.patch.object(capture.resource_sampler, "collect_once", return_value=idle),
                contextlib.redirect_stdout(io.StringIO()) as out_buf,
            ):
                rc = capture.cmd_headless(args)
            self.assertEqual(rc, 0)
            self.assertIn("Backlog O2 (GPU, open) Measure: Static spawn: `render_graph`", out_buf.getvalue())
            mocked_run.assert_called_once()
            meta = json.loads((out_dir / "capture_meta.json").read_text(encoding="utf-8"))
            self.assertEqual(meta["executable_sha256"], hashlib.sha256(b"headless-binary").hexdigest())
            self.assertEqual(meta["idle_baseline"], idle)
            self.assertEqual(meta["args"], ["--uncapped"])
            self.assertEqual(meta["backlog_item"], "O2")
            self.assertEqual(meta["backlog_measure"], "Static spawn: `render_graph`")
            self.assertEqual(meta["exit_code"], 0)

            bad_args = argparse.Namespace(**{**vars(args), "item": "O999"})
            with contextlib.redirect_stderr(io.StringIO()) as err_buf:
                rc_bad = capture.cmd_headless(bad_args)
            self.assertEqual(rc_bad, 1)
            self.assertIn("unknown backlog item", err_buf.getvalue())


class TestSkillDocs(unittest.TestCase):
    def test_skill_files_and_referenced_scripts_exist(self):
        canonical = SKILL_DIR / "SKILL.md"
        claude_stub = REPO / ".claude" / "skills" / "hl2-performance-trace" / "SKILL.md"
        self.assertTrue(canonical.is_file())
        self.assertTrue(claude_stub.is_file())
        canonical_text = canonical.read_text(encoding="utf-8")
        stub_text = claude_stub.read_text(encoding="utf-8")
        self.assertIn("name: hl2-performance-trace", canonical_text)
        self.assertIn("name: hl2-performance-trace", stub_text)
        self.assertIn(".agents/skills/hl2-performance-trace/SKILL.md", stub_text)

        refs = set(
            re.findall(
                r"\.agents/skills/hl2-performance-trace/scripts/([A-Za-z0-9_.-]+\.py)",
                canonical_text,
            )
        )
        expected = {
            "capture.py",
            "trace_summary.py",
            "trace_waste.py",
            "resource_sampler.py",
            "metal_gpu.py",
            "sample_summary.py",
        }
        self.assertEqual(refs, expected)
        for name in refs:
            self.assertTrue((SCRIPTS_DIR / name).is_file(), name)


if __name__ == "__main__":
    unittest.main()
