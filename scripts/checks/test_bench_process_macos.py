import ctypes
import unittest
from bench_process_macos import ProcessUsage, Usage, swap_usage


class ProcessMemoryTests(unittest.TestCase):
    def test_footprint_uses_footprint_field_not_resident_memory(self):
        probe = ProcessUsage.__new__(ProcessUsage)
        def read(pid, flavor, ptr):
            self.assertEqual((pid, flavor), (123, 2))
            usage = ctypes.cast(ptr, ctypes.POINTER(Usage)).contents
            usage.resident_size, usage.phys_footprint = 100, 300
            usage.diskio_bytesread = 90
            return 0
        probe.read = read
        self.assertEqual(probe.sample(123), {"game_phys_footprint_bytes": 300,
                         "game_disk_read_bytes_counter": 90, "game_disk_write_bytes_counter": 0})
        probe.read = lambda *args: -1
        self.assertEqual(probe.sample(123), {})
        self.assertEqual(ctypes.sizeof(Usage), 160)

    def test_swap_occupancy_is_not_a_cumulative_counter(self):
        self.assertEqual(swap_usage('total = 5120.00M used = 4102.25M free = 1017.75M (encrypted)'),
                         {"swap_total_bytes": 5120*1024**2, "swap_used_bytes": 4102.25*1024**2,
                          "swap_free_bytes": 1017.75*1024**2})
        self.assertEqual(swap_usage('unavailable'), {})
