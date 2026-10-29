"""Bounded benchmark telemetry, alignment and explicitly observational resource ratios."""
import json
import math
import os
import platform
import statistics
import threading
import time
try:
    import resource
except ImportError:
    resource = None


def observer_cpu():
    if resource is None:
        return None
    return sum(getattr(resource.getrusage(kind), field) for kind in
               (resource.RUSAGE_SELF, resource.RUSAGE_CHILDREN) for field in ("ru_utime", "ru_stime"))


class Unavailable:
    def metadata(self):
        return {"provider": "unavailable", "logical_cpus": os.cpu_count(), "memory_model": "unknown",
                "dedicated_vram_available_bytes": None}
    def sample(self):
        return {}
    def thermal(self):
        return {}


def known_interference(samples):
    """Known competing workloads; missing counters do not establish idleness."""
    return [key for key in ("competing_build_processes", "other_game_processes")
            if any((s.get(key) or 0) > 0 for s in samples)]


def correlation(pairs):
    pairs = [(x, y) for x, y in pairs if x is not None and y is not None
             and math.isfinite(x) and math.isfinite(y)]
    if len(pairs) < 6:
        return {"samples": len(pairs), "pearson_r": None}
    x, y = zip(*pairs)
    dx, dy = [v-statistics.mean(x) for v in x], [v-statistics.mean(y) for v in y]
    denominator = math.sqrt(sum(v*v for v in dx)*sum(v*v for v in dy))
    return {"samples": len(pairs), "pearson_r": sum(a*b for a, b in zip(dx, dy))/denominator if denominator else None}


def means(samples):
    keys = {k for s in samples for k in s if k not in ("unix_seconds", "sampler_wall_ms", "sampler_cpu_seconds", "memory_pressure_level")}
    return {k: statistics.mean(values) for k in keys if
            (values := [s[k] for s in samples if isinstance(s.get(k), (int, float))])}


def align(samples, windows):
    result = []
    for window in windows:
        start, end = window.get("start_unix_seconds"), window.get("end_unix_seconds")
        if start is None or end is None:
            continue
        selected = [s for s in samples if start <= s["unix_seconds"] < end]
        if selected:
            result.append({**window, "environment": means(selected), "resource_samples": len(selected)})
    return result


def throughput(fps, width, height, resources):
    rate = fps*width*height/1e6
    cpu, gpu = resources.get("game_cpu_percent_one_core"), resources.get("gpu_busy_percent")
    return {"frames_per_second": fps, "megapixels_per_second": rate,
            "frames_per_game_cpu_second": fps/(cpu/100) if cpu and cpu > 0 else None,
            "game_cpu_ms_per_frame": 10*cpu/fps if cpu is not None and fps > 0 else None,
            "system_gpu_busy_ms_per_frame_proxy": 10*gpu/fps if gpu is not None and fps > 0 else None,
            "power_watts": None, "energy_joules_per_frame": None,
            "megapixels_per_system_gpu_busy_second_proxy": rate/(gpu/100) if gpu and gpu > 0 else None}


def report(samples, baseline, frame, metadata, size):
    windows = frame.get("frame_windows", {}).get("windows", [])
    aligned = align(samples, windows)
    active = [s for s in samples if any(w.get("start_unix_seconds", math.inf) <= s["unix_seconds"]
                < w.get("end_unix_seconds", -math.inf) for w in windows
                if w.get("start_unix_seconds") is not None and w.get("end_unix_seconds") is not None)]
    average = means(active)
    dropped = frame.get("frame_windows", {}).get("dropped_windows", 0)
    retained_frames = sum(w.get("frames", 0) for w in windows)
    retained_seconds = sum(w.get("elapsed_seconds", 0) for w in windows)
    fps = retained_frames/retained_seconds if retained_seconds > 0 else frame["mean_fps"]
    issues = []
    if not active:
        issues.append("no timestamp-aligned resource samples")
    for key in ("cpu_busy_percent", "gpu_busy_percent"):
        if sum(s.get(key) is not None for s in active) < 3:
            issues.append(key + " unavailable or insufficient")
    issues.extend(known_interference(active))
    if any(s.get("memory_pressure_level", 1) != 1 for s in active):
        issues.append("memory pressure during measured frames")
    for counter in ("swapout_bytes_counter", "pageout_bytes_counter"):
        values = [s[counter] for s in active if counter in s]
        if len(values) > 1 and values[-1] > values[0]:
            issues.append(counter + " increased")
    return {"hardware": metadata, "baseline": means(baseline), "active_mean": average,
            "active_min_available_memory_bytes": min((s["memory_available_estimate_bytes"] for s in active
                if "memory_available_estimate_bytes" in s), default=None),
            "active_max_game_rss_bytes": max((s["game_rss_bytes"] for s in active if "game_rss_bytes" in s), default=None),
            "memory_pressure_levels": sorted({s["memory_pressure_level"] for s in active if "memory_pressure_level" in s}),
            "quality_flags": issues, "aligned_windows": aligned,
            "resource_sample_count": len(active),
            "sampler_mean_wall_ms": statistics.mean([s["sampler_wall_ms"] for s in samples]) if samples else None,
            "sampler_active_cpu_seconds": sum(s.get("sampler_cpu_seconds") or 0 for s in active) if any(s.get("sampler_cpu_seconds") is not None for s in active) else None,
            "sampler_active_cpu_samples": sum(s.get("sampler_cpu_seconds") is not None for s in active),
            "throughput_window": "retained frame windows" if dropped else "all post-warm-up frames",
            "throughput": throughput(fps, *size, average),
            "within_run_associations_with_frame_ms": {key: correlation([
                (w["environment"].get(key), w["mean_frame_ms"]) for w in aligned]) for key in
                ("cpu_busy_percent", "gpu_busy_percent", "memory_available_estimate_bytes")},
            "caveats": ["CPU/GPU load includes the benchmark; correlation is not causation.",
                "Driver GPU busy percent is an occupancy proxy, not available FLOPS or an isolated GPU duration.",
                "GPU throughput proxy includes other GPU users; do not compare it across different GPUs as compute efficiency.",
                "macOS available memory is a free+inactive+speculative page estimate, not an allocation guarantee.",
                "CPU samples average the preceding sampling interval; GPU driver averaging interval is unspecified.",
                "Sampler CPU includes its child commands; samples containing a game exit are excluded from CPU accounting.",
                "These descriptive correlations have no significance test; adjacent windows are not independent.",
                "Unsupported fields remain absent/null; clock rates and available FLOPS are not measured."]}


def pair_environment(retail, advanced):
    issues = sorted(set(retail["quality_flags"] + advanced["quality_flags"]))
    for key in ("cpu", "logical_cpus", "memory_total_bytes", "memory_model", "adapter", "backend"):
        if retail["hardware"].get(key) != advanced["hardware"].get(key):
            issues.append("hardware differs: " + key)
    a, b = (r["baseline"].get("memory_available_estimate_bytes") for r in (retail, advanced))
    total = retail["hardware"].get("memory_total_bytes", 0)
    if a is not None and b is not None and abs(a-b) > max(512*1024**2, total*0.10):
        issues.append("baseline available memory differs materially")
    for key in ("cpu_busy_percent", "gpu_busy_percent"):
        a, b = retail["baseline"].get(key), advanced["baseline"].get(key)
        if a is not None and b is not None and abs(a-b) > 10:
            issues.append("baseline " + key + " differs by more than 10 percentage points")
    return {"comparable": not issues, "flags": issues,
            "both_baselines_quiet": all(r["hardware"].get("quiet_baseline_observed") is True for r in (retail, advanced))}


class Monitor:
    def __init__(self, executable, interval=1.):
        if not math.isfinite(interval) or not 0 < interval <= 5:
            raise ValueError("telemetry interval must be positive and at most five seconds")
        self.interval = interval
        self.probe = Unavailable()
        self.errors = []
        if platform.system() == "Darwin":
            try:
                from bench_environment_macos import Probe
                self.probe = Probe(executable)
            except (OSError, AttributeError) as error:
                self.errors.append(type(error).__name__)
        self.metadata = {"os": platform.system(), "release": platform.release(),
                         "machine": platform.machine(), **self.probe.metadata(),
                         "thermal_limits_before": self.probe.thermal()}
        self.samples, self.baseline = [], []
        self.stop = threading.Event()
        self.epoch, self.clock = time.time(), time.monotonic()

    def sample(self):
        start = time.monotonic()
        cpu_start = observer_cpu()
        try:
            data = self.probe.sample()
        except (OSError, ValueError, TypeError) as error:
            self.errors.append(type(error).__name__)
            data = {}
        end = time.monotonic()
        cpu_end = observer_cpu()
        cpu = cpu_end-cpu_start if cpu_start is not None and cpu_end is not None else None
        # The main thread also waits for the game: its exit may update child rusage here.
        if cpu is not None and cpu > 2*(end-start)+0.01:
            cpu = None
        return {"sampler_cpu_seconds": cpu,"unix_seconds": self.epoch+(start+end)/2-self.clock,
                "sampler_wall_ms": (end-start)*1000, **data}

    def __enter__(self):
        self.primer = self.sample()
        # Prefer two quiet, current samples; never wait indefinitely on a shared host.
        deadline = time.monotonic()+5
        while True:
            self.stop.wait(self.interval)
            self.baseline.append(self.sample())
            recent = self.baseline[-2:]
            quiet = len(recent) == 2 and all(
                s.get("cpu_busy_percent", 101) <= 20 and s.get("gpu_busy_percent", 101) <= 15
                and s.get("memory_pressure_level", 1) == 1 and not s.get("competing_build_processes", 0)
                and not s.get("other_game_processes", 0) for s in recent)
            if quiet or time.monotonic() >= deadline or type(self.probe) is Unavailable:
                break
        self.baseline_history = self.baseline
        self.baseline = self.baseline[-2:]
        self.metadata["quiet_baseline_observed"] = quiet
        self.worker = threading.Thread(target=self.collect, daemon=True)
        self.worker.start()
        return self

    def collect(self):
        while not self.stop.is_set():
            start = time.monotonic()
            self.samples.append(self.sample())
            self.stop.wait(max(0., self.interval-(time.monotonic()-start)))

    def __exit__(self, *_):
        self.stop.set()
        self.worker.join()

    def finish(self, path, frame, size):
        result = report(self.samples, self.baseline, frame, self.metadata, size)
        result["sampler_errors"] = self.errors
        result["thermal_limits_after"] = self.probe.thermal()
        if self.errors:
            result["quality_flags"].append("sampler errors")
        if any(v < 100 for limits in (self.metadata["thermal_limits_before"], result["thermal_limits_after"])
               for k, v in limits.items() if k in ("CPU_Speed_Limit", "CPU_Scheduler_Limit")):
            result["quality_flags"].append("reported CPU thermal limit")
        path.write_text(json.dumps({"summary": result, "baseline": self.baseline,
                                   "primer": self.primer, "baseline_history": self.baseline_history, "samples": self.samples}, indent=2), encoding="utf-8")
        return result
