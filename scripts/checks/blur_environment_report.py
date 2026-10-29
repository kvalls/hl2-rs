"""Readable resource/output ratios from one benchmark summary (no new measurements)."""
import json
from pathlib import Path


def markdown(results):
    lines = ["# Blur resource and output measurements", "",
             "Host utilisation includes the workload itself. Correlations are descriptive, not causal.",
             "GPU busy-time throughput is a system-wide proxy, not game-only GPU time or FLOPS.",
             "Power and joules/frame are not sampled; utilisation is not an energy measurement.",
             "Memory columns use macOS free+inactive+speculative pages; dedicated VRAM is unavailable on unified memory.", "",
             "| Scene / mode | FPS | Frame ms | p95 ms | CPU busy % | GPU busy % | Min available GiB | Mpixel/s | Frames / game CPU-s | Mpixel / system GPU-busy-s proxy | Flags |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"]
    value = lambda x: f"{x:.2f}" if isinstance(x, (int, float)) else "unavailable"
    ratios = []
    for scene, pair in results.items():
        for mode in ("retail", "advanced"):
            for index, run in enumerate(pair[mode]):
                env = run["environment"]
                resources, rate = env["active_mean"], env["throughput"]
                available = env["active_min_available_memory_bytes"]
                cells = [f"{scene} / {mode} {index+1}", value(1000/run["mean_frame_ms"] if run["mean_frame_ms"] else None), value(run["mean_frame_ms"]),
                         value(run["p95_frame_ms_upper_bound"]), value(resources.get("cpu_busy_percent")),
                         value(resources.get("gpu_busy_percent")), value(available/1024**3 if available is not None else None),
                         value(rate["megapixels_per_second"]), value(rate["frames_per_game_cpu_second"]),
                         value(rate.get("megapixels_per_system_gpu_busy_second_proxy")),
                         "; ".join(env["quality_flags"]) or "none"]
                lines.append("| " + " | ".join(cells) + " |")
        if 'iteration_statistics' in pair:
            ratios += ['', f"{scene} repeated-run medians/ranges: " + json.dumps(pair['iteration_statistics']),
                       'Paired frame-time ratios: ' + json.dumps(pair['paired_frame_time_ratios'])]
        ratios += ["", f"{scene}: Advanced/Enabled frame-time ratio **{pair['frame_time_ratio_advanced_to_retail']:.3f}**.",
                  "Environment checks: " + json.dumps(pair["environment_pairs"]), ""]
    lines += ["", "## Paired costs", "", *ratios, "## Within-run associations", "", "Pearson r against mean frame milliseconds; adjacent windows are not independent. No significance test is claimed.", ""]
    for scene, pair in results.items():
        for mode in ("retail", "advanced"):
            for index, run in enumerate(pair[mode]):
                env = run["environment"]
                text = "; ".join(f"{key}: r={value(c['pearson_r'])}, n={c['samples']}" for key, c in
                                 env["within_run_associations_with_frame_ms"].items())
                lines.append(f"- {scene} / {mode} {index+1}: {text}")
    return "\n".join(lines) + "\n"


def write(path, results):
    Path(path).write_text(markdown(results), encoding="utf-8")


if __name__ == '__main__':
    import sys
    source = Path(sys.argv[1])
    write(source.with_name('resource-report.md'), json.loads(source.read_text(encoding='utf-8')))
