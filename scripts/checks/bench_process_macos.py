"""Read-only process footprint from macOS rusage_info_v2; no runtime dependency."""
import ctypes
import ctypes.util
import re


class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
        "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups", "pageins",
        "wired_size", "resident_size", "phys_footprint", "proc_start_abstime",
        "proc_exit_abstime", "child_user_time", "child_system_time", "child_pkg_idle_wkups",
        "child_interrupt_wkups", "child_pageins", "child_elapsed_abstime",
        "diskio_bytesread", "diskio_byteswritten")]


class ProcessUsage:
    def __init__(self):
        lib = ctypes.CDLL(ctypes.util.find_library("proc"))
        self.read = lib.proc_pid_rusage
        self.read.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.read.restype = ctypes.c_int

    def sample(self, pid):
        usage = Usage()
        if self.read(pid, 2, ctypes.byref(usage)) != 0:
            return {}  # Exited/inaccessible processes are unknown, not zero.
        return {"game_phys_footprint_bytes": usage.phys_footprint,
                "game_disk_read_bytes_counter": usage.diskio_bytesread,
                "game_disk_write_bytes_counter": usage.diskio_byteswritten}


def swap_usage(raw):
    text = raw.decode() if isinstance(raw, bytes) else raw
    values = re.findall(r"(total|used|free)\s*=\s*([\d.]+)([MGT])", text)
    return {"swap_" + key + "_bytes": round(float(value) * 1024 ** {"M": 2, "G": 3, "T": 4}[unit])
            for key, value, unit in values}
