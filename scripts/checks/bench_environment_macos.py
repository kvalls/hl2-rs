"""Read-only macOS host counters. Driver utilisation is not a FLOPS measurement."""
import ctypes
import ctypes.util
import os
import plistlib
import re
import subprocess
import time
from pathlib import Path
from bench_process_macos import ProcessUsage, swap_usage


def command(*args):
    try:
        return subprocess.check_output(args, stderr=subprocess.DEVNULL, timeout=3)
    except (OSError, subprocess.SubprocessError):
        return None


def vm_pages(raw):
    text = raw.decode() if isinstance(raw, bytes) else raw
    match = re.search(r"page size of (\d+) bytes", text)
    if not match:
        return {}
    size = int(match[1])
    pages = {k.strip('"'): int(v) for k, v in re.findall(r'^"?([^:\n]+?)"?:\s+(\d+)\.', text, re.M)}
    return {"memory_free_bytes": pages.get("Pages free", 0)*size,
            "memory_available_estimate_bytes": sum(pages.get(k, 0) for k in
                ("Pages free", "Pages inactive", "Pages speculative"))*size,
            "memory_compressed_bytes": pages.get("Pages occupied by compressor", 0)*size,
            "swapout_bytes_counter": pages.get("Swapouts", 0)*size,
            "pageout_bytes_counter": pages.get("Pageouts", 0)*size}


def gpu_stats(raw):
    """One Apple GPU is supported; multiple devices are explicitly ambiguous."""
    devices = []
    def visit(node):
        if isinstance(node, list):
            for child in node:
                visit(child)
        elif isinstance(node, dict):
            if isinstance(node.get("PerformanceStatistics"), dict):
                devices.append(node["PerformanceStatistics"])
            for key, child in node.items():
                if key != "PerformanceStatistics" and isinstance(child, (dict, list)):
                    visit(child)
    visit(plistlib.loads(raw))
    result = {"gpu_counter_device_count": len(devices)}
    if len(devices) != 1:
        return result
    keys = {"Device Utilization %": "gpu_busy_percent", "Renderer Utilization %": "gpu_renderer_percent",
            "Tiler Utilization %": "gpu_tiler_percent", "Alloc system memory": "gpu_driver_allocated_system_bytes",
            "In use system memory": "gpu_in_use_system_bytes"}
    for source, target in keys.items():
        value = devices[0].get(source)
        if isinstance(value, (float, int)) and (not target.endswith("percent") or 0 <= value <= 100):
            result[target] = value
    if "gpu_busy_percent" in result:
        result["gpu_headroom_percent_proxy"] = 100-result["gpu_busy_percent"]
    return result


def cpu_percent(previous, current):
    if previous is None:
        return None
    delta = [(b-a) % (1 << 32) for a, b in zip(previous, current)]
    total = sum(delta)
    return 100*(total-delta[2])/total if total else None


def cpu_seconds(text):
    days, _, clock = text.partition("-")
    seconds = int(days)*86400 if clock else 0
    clock = clock or text
    for index, part in enumerate(reversed(clock.split(":"))):
        seconds += float(part)*60**index
    return seconds


class Probe:
    def __init__(self, executable):
        self.executable = str(Path(executable).resolve())
        self.lib = ctypes.CDLL(ctypes.util.find_library("System"))
        self.lib.mach_host_self.restype = ctypes.c_uint
        self.lib.host_statistics.argtypes = [ctypes.c_uint, ctypes.c_int,
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]
        self.host = self.lib.mach_host_self()
        self.usage = ProcessUsage()
        self.cpu = None
        self.process = None
        self.previous_time = None

    def metadata(self):
        result = {"provider": "macos-host/ioreg", "logical_cpus": os.cpu_count(),
                  "memory_model": "unified" if os.uname().machine == "arm64" else "unknown",
                  "dedicated_vram_available_bytes": None}
        for name, key in (("hw.memsize", "memory_total_bytes"), ("hw.physicalcpu", "physical_cpus"),
                          ("machdep.cpu.brand_string", "cpu")):
            raw = command("sysctl", "-n", name)
            if raw:
                value = raw.decode().strip()
                result[key] = int(value) if value.isdigit() else value
        return result

    def thermal(self):
        raw = command("pmset", "-g", "therm")
        values = re.findall(r"(CPU_Speed_Limit|CPU_Scheduler_Limit|CPU_Available_CPUs)\s*=\s*(\d+)", raw.decode() if raw else "")
        return dict((key, int(value)) for key, value in values)

    def sample(self):
        result = {}
        current = (ctypes.c_uint*4)()
        count = ctypes.c_uint(4)
        now = time.monotonic()
        if self.lib.host_statistics(self.host, 3, current, ctypes.byref(count)) == 0:
            result["cpu_busy_percent"] = cpu_percent(self.cpu, current)
            self.cpu = tuple(current)
            if result["cpu_busy_percent"] is not None:
                result["cpu_idle_percent"] = 100-result["cpu_busy_percent"]
        for args, decode in ((('vm_stat',), vm_pages),
                             (('ioreg', '-r', '-c', 'IOAccelerator', '-d', '1', '-a'), gpu_stats)):
            raw = command(*args)
            if raw:
                try:
                    result.update(decode(raw))
                except (ValueError, TypeError, plistlib.InvalidFileException):
                    pass
        pressure = command("sysctl", "-n", "kern.memorystatus_vm_pressure_level")
        if pressure and pressure.strip().isdigit():
            result["memory_pressure_level"] = int(pressure)
        swap = command("sysctl", "-n", "vm.swapusage")
        if swap:
            result.update(swap_usage(swap))
        processes = command('ps', '-A', '-o', 'pid=,time=,rss=,comm=')
        if processes:
            result.update(self.processes(processes.decode(), now))
        self.previous_time = now
        return result

    def processes(self, text, now):
        result = {"competing_build_processes": 0, "other_game_processes": 0}
        for line in text.splitlines():
            parts = line.split(None, 3)
            if len(parts) != 4:
                continue
            pid, cpu, rss, path = parts
            name = Path(path).name
            if name in ("cargo", "rustc", "cargo-clippy", "clippy-driver"):
                result["competing_build_processes"] += 1
            if path == self.executable:
                result["game_rss_bytes"] = int(rss)*1024
                result.update(self.usage.sample(int(pid)))
                total = cpu_seconds(cpu)
                if self.process and self.process[0] == pid and self.previous_time is not None:
                    result["game_cpu_percent_one_core"] = 100*(total-self.process[1])/max(now-self.previous_time, 1e-9)
                self.process = (pid, total)
            elif name in ("hl2-bevy", "hl2-bevy.exe"):
                result["other_game_processes"] += 1
        return result
