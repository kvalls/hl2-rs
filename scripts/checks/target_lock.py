"""Coordinate local stale pruning with Cargo without waiting on active builds."""
from contextlib import contextmanager, ExitStack
import os
from pathlib import Path
import subprocess
import sys


def darwin_filesystem(target_dir):
    """Read the 64-bit Darwin statfs record without disk-management services."""
    import ctypes

    class StatFs(ctypes.Structure):
        _fields_ = [
            ("block_size", ctypes.c_uint32), ("io_size", ctypes.c_int32),
            ("counts", ctypes.c_uint64 * 5), ("fsid", ctypes.c_int32 * 2),
            ("owner", ctypes.c_uint32), ("kind", ctypes.c_uint32),
            ("flags", ctypes.c_uint32), ("subtype", ctypes.c_uint32),
            ("name", ctypes.c_char * 16), ("mount", ctypes.c_char * 1024),
            ("source", ctypes.c_char * 1024), ("extended_flags", ctypes.c_uint32),
            ("reserved", ctypes.c_uint32 * 7),
        ]

    libc = ctypes.CDLL(None, use_errno=True)
    statfs = getattr(libc, "statfs$INODE64", libc.statfs)
    statfs.argtypes = [ctypes.c_char_p, ctypes.POINTER(StatFs)]
    statfs.restype = ctypes.c_int
    data = StatFs()
    if statfs(os.fsencode(target_dir), ctypes.byref(data)) != 0:
        raise OSError(ctypes.get_errno(), "cannot inspect target filesystem")
    return data.name.decode("ascii", errors="replace").lower()


def local_filesystem(target_dir):
    """Cargo can omit locks on network mounts; prune only known local filesystems."""
    if sys.platform == "darwin":
        return darwin_filesystem(target_dir) in {"apfs", "hfs"}
    elif sys.platform.startswith("linux"):
        command = ["stat", "-f", "-c", "%T", str(target_dir)]
    else:
        return False
    kind = subprocess.check_output(command, text=True, stderr=subprocess.DEVNULL,
                                   timeout=5).strip().lower()
    return kind in {"apfs", "hfs", "hfs+", "ext2/ext3", "ext4", "xfs", "btrfs",
                    "tmpfs", "overlayfs", "zfs"}


@contextmanager
def cargo_locks(target_dir):
    """Hold Cargo's debug/release compatibility locks until pruning finishes.

    Current Cargo holds shared .cargo-lock locks; older Cargo holds them exclusively.
    Exclusive flock coordinates with both on POSIX. Never unlink these lock files.
    Unsupported platforms/filesystems and busy targets must skip pruning entirely.
    """
    with ExitStack() as handles:
        try:
            import fcntl

            if not local_filesystem(target_dir):
                raise OSError("unverified or network filesystem")
            for profile in ("debug", "release"):
                folder = Path(target_dir) / profile
                folder.mkdir(parents=True, exist_ok=True)
                lock = handles.enter_context((folder / ".cargo-lock").open("a+b"))
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (ImportError, OSError, subprocess.SubprocessError) as error:
            handles.close()
            print(f"Skipping stale cleanup: Cargo target busy or compatible locking unavailable ({error}).")
            yield False
            return
        yield True
