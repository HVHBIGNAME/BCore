"""Owned subprocesses and listener identity checks for the queued protocol probe."""
from __future__ import annotations

import ctypes
from ctypes import wintypes
import math
import os
from pathlib import Path
import socket
import struct
import subprocess
import threading
import time

import native


def reserve_port(requested: int = 0) -> socket.socket:
    reservation = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        if os.name == "nt":
            reservation.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        reservation.bind(("127.0.0.1", requested))
    except BaseException:
        reservation.close()
        raise
    return reservation


def listeners() -> list[dict]:
    if os.name != "nt":
        raise RuntimeError("listener PID attribution currently requires Windows GetExtendedTcpTable")
    api = ctypes.WinDLL("iphlpapi", use_last_error=True)
    query = api.GetExtendedTcpTable
    query.argtypes = (ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD), wintypes.BOOL,
                      wintypes.ULONG, ctypes.c_int, wintypes.ULONG)
    query.restype = wintypes.DWORD
    size = wintypes.DWORD()
    result = query(None, ctypes.byref(size), False, socket.AF_INET, 3, 0)
    if result not in (0, 122):
        raise OSError(result, "GetExtendedTcpTable size query failed")
    for _ in range(4):
        buffer = ctypes.create_string_buffer(size.value)
        result = query(buffer, ctypes.byref(size), False, socket.AF_INET, 3, 0)
        if result == 122:
            continue
        if result:
            raise OSError(result, "GetExtendedTcpTable failed")
        count = struct.unpack_from("<I", buffer)[0]
        if 4 + 24 * count > len(buffer):
            raise RuntimeError("truncated TCP owner table")
        rows = []
        for index in range(count):
            state, address, port, _, _, pid = struct.unpack_from("<6I", buffer, 4 + 24 * index)
            if state != 2:
                raise RuntimeError("non-listener in TCP listener table")
            rows.append({"host": socket.inet_ntoa(struct.pack("<I", address)),
                         "port": struct.unpack(">H", struct.pack("<I", port)[:2])[0], "pid": pid})
        return rows
    raise RuntimeError("TCP owner table changed repeatedly")


def require_listener(pid: int, port: int) -> dict:
    rows = listeners()
    on_port = [row for row in rows if row["port"] == port]
    expected = {"host": "127.0.0.1", "port": port, "pid": pid}
    if on_port != [expected]:
        raise RuntimeError(f"loopback listener ownership differs: {on_port}, expected {expected}")
    return {"method": "GetExtendedTcpTable/TCP_TABLE_OWNER_PID_LISTENER",
            "listener": expected, "owned_listeners": [row for row in rows if row["pid"] == pid]}


class WindowsJob:
    def __init__(self):
        if os.name != "nt":
            raise RuntimeError("owned process-tree cleanup currently requires Windows Job Objects")
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = (ctypes.c_void_p, wintypes.LPCWSTR)
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.SetInformationJobObject.argtypes = (wintypes.HANDLE, ctypes.c_int,
                                                    ctypes.c_void_p, wintypes.DWORD)
        self.api.SetInformationJobObject.restype = wintypes.BOOL
        self.api.AssignProcessToJobObject.argtypes = (wintypes.HANDLE, wintypes.HANDLE)
        self.api.AssignProcessToJobObject.restype = wintypes.BOOL
        self.api.QueryInformationJobObject.argtypes = (wintypes.HANDLE, ctypes.c_int,
                                                      ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p)
        self.api.QueryInformationJobObject.restype = wintypes.BOOL
        self.api.CloseHandle.argtypes = (wintypes.HANDLE,)
        self.api.CloseHandle.restype = wintypes.BOOL

        class BasicLimits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                        ("flags", wintypes.DWORD), ("minimum_working_set", ctypes.c_size_t),
                        ("maximum_working_set", ctypes.c_size_t), ("active_process_limit", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling_class", wintypes.DWORD)]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("basic", BasicLimits), ("io", ctypes.c_ulonglong * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t)]

        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            self.close()
            raise error

    def attach(self, process: subprocess.Popen) -> None:
        if not self.api.AssignProcessToJobObject(self.handle, wintypes.HANDLE(int(process._handle))):
            raise ctypes.WinError(ctypes.get_last_error())

    def active_processes(self) -> int:
        # JOBOBJECT_BASIC_ACCOUNTING_INFORMATION: four LARGE_INTEGERs, then four DWORDs.
        info = ctypes.create_string_buffer(48)
        if not self.api.QueryInformationJobObject(self.handle, 1, info, len(info), None):
            raise ctypes.WinError(ctypes.get_last_error())
        return struct.unpack_from("<I", info, 40)[0]

    def close(self) -> None:
        if self.handle:
            handle, self.handle = self.handle, None
            if not self.api.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())


class OwnedProcess:
    def __init__(self, command: list[str], cwd: Path, output: Path, label: str,
                 environment: dict[str, str], log_limit: int):
        self.command, self.cwd, self.output, self.label = command, cwd, output, label
        self.environment, self.log_limit = environment, log_limit
        self.process = None
        self.job = None
        self.threads = []
        self.failure = threading.Event()
        self.errors = []
        self.tail = bytearray()
        self.tail_lock = threading.Lock()
        self.counts = {"stdout": 0, "stderr": 0}
        self.retained = {"stdout": 0, "stderr": 0}
        self.started = None
        self.cleanup = {"state": "not_started", "attempts": []}

    def start(self) -> None:
        self.job = WindowsJob()
        self.started = time.time()
        try:
            native.write_json(self.output / f"{self.label}.command.json", {
                "argv": self.command, "windows_command_line": subprocess.list2cmdline(self.command),
                "cwd": str(self.cwd), "environment": self.environment, "inherit_other_environment": False,
                "started_unix": self.started, "log_limit_bytes_per_stream": self.log_limit})
            self.process = subprocess.Popen(self.command, cwd=self.cwd, env=self.environment,
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
            self.job.attach(self.process)
            for name in ("stdout", "stderr"):
                thread = threading.Thread(target=self._drain, args=(name,), daemon=True)
                self.threads.append(thread)
                thread.start()
            self.cleanup["state"] = "running"
        except BaseException:
            self.close(0)
            raise

    def _drain(self, name: str) -> None:
        source = getattr(self.process, name)
        try:
            with (self.output / f"{self.label}.{name}.log").open("xb") as destination:
                while True:
                    block = source.read1(65536)
                    if not block:
                        break
                    self.counts[name] += len(block)
                    keep = block[:max(0, self.log_limit - self.retained[name])]
                    destination.write(keep)
                    destination.flush()
                    self.retained[name] += len(keep)
                    if name == "stdout":
                        with self.tail_lock:
                            self.tail.extend(block)
                            del self.tail[:-65536]
                    if len(keep) != len(block):
                        self.errors.append(f"{name} exceeded bounded log limit")
                        self.failure.set()
        except (OSError, ValueError) as error:
            self.errors.append(f"{name} log drain: {error}")
            self.failure.set()
        finally:
            source.close()

    def stdout_tail(self) -> bytes:
        with self.tail_lock:
            return bytes(self.tail)

    def check(self) -> None:
        if self.failure.is_set():
            raise RuntimeError("; ".join(self.errors))
        if self.process.poll() is not None:
            raise RuntimeError(f"{self.label} exited early: {self.process.returncode}")

    def write_stdin(self, value: bytes) -> None:
        self.check()
        self.process.stdin.write(value)
        self.process.stdin.flush()
        self.cleanup["attempts"].append({"method": "stdin", "bytes_hex": value.hex(), "unix": time.time()})

    def wait(self, timeout: float) -> int:
        deadline = time.monotonic() + timeout
        while self.process.poll() is None:
            if self.failure.is_set():
                raise RuntimeError("; ".join(self.errors))
            if time.monotonic() >= deadline:
                raise TimeoutError(f"{self.label} exceeded {timeout} seconds")
            time.sleep(0.05)
        return self.process.returncode

    def close(self, grace: float, *, port: int | None = None) -> dict:
        errors, forced = [], False
        if self.process is not None:
            for method, timeout in (("wait", grace), ("terminate", 5), ("kill", 5)):
                if self.process.poll() is not None:
                    break
                try:
                    if method != "wait":
                        forced = True
                        getattr(self.process, method)()
                    self.cleanup["attempts"].append({"method": method, "unix": time.time()})
                    self.process.wait(timeout=timeout)
                except subprocess.TimeoutExpired:
                    self.cleanup["attempts"][-1]["timed_out"] = True
                except OSError as error:
                    errors.append(f"{method}: {error}")
            if self.process.stdin is not None:
                try:
                    self.process.stdin.close()
                except OSError as error:
                    errors.append(f"close stdin: {error}")
        active = None
        if self.job is not None:
            try:
                active = self.job.active_processes()
                if active:
                    forced = True
                self.job.close()
            except OSError as error:
                errors.append(f"job cleanup: {error}")
        for thread in self.threads:
            thread.join(timeout=5)
        if any(thread.is_alive() for thread in self.threads):
            errors.append("log drain did not finish")
        if self.process is not None and self.process.poll() is None:
            errors.append("owned root exit unknown")
        if active not in (0, None):
            errors.append(f"job still had {active} processes before close; descendant exits unobserved")
        if port is not None:
            try:
                self.cleanup["listeners_after"] = [row for row in listeners() if row["port"] == port]
                if self.cleanup["listeners_after"]:
                    errors.append("selected port still has a listener")
                with reserve_port(port):
                    self.cleanup["port_exclusively_rebound"] = True
            except (OSError, RuntimeError) as error:
                errors.append(f"port release: {error}")
        self.cleanup.update(state="cleanup_unknown" if errors else "clean", forced=forced,
                            errors=errors, root_exit_code=self.process.poll() if self.process is not None else None,
                            pid=self.process.pid if self.process is not None else None, started_unix=self.started,
                            job_active_before_close=active, observed_log_bytes=self.counts,
                            retained_log_bytes=self.retained, log_errors=self.errors, ended_unix=time.time())
        native.write_json(self.output / f"{self.label}.result.json", self.cleanup)
        return self.cleanup


def wait_ready(owner: OwnedProcess, port: int, timeout: float, *, native_server: bool) -> dict:
    deadline = time.monotonic() + timeout
    marker = b"Done (" if native_server else b"listening on 127.0.0.1:"
    while time.monotonic() < deadline:
        owner.check()
        rows = listeners()
        on_port = [row for row in rows if row["port"] == port]
        if on_port:
            identity = require_listener(owner.process.pid, port)
            if marker in owner.stdout_tail():
                owner.check()
                identity.update(ready_unix=time.time(), startup_marker=marker.decode("ascii"),
                                readiness_scope="startup_marker_and_owned_listener", status_availability="not_probed")
                return identity
        time.sleep(0.1)
    raise TimeoutError(f"server startup exceeded {timeout} seconds")


def settle_listener(owner: OwnedProcess, port: int, seconds: float) -> dict:
    if not math.isfinite(seconds) or not 0 < seconds <= 5:
        raise ValueError("status retry settle must be finite within (0, 5] seconds")
    owner.check()
    before = require_listener(owner.process.pid, port)
    started, started_unix = time.monotonic(), time.time()
    deadline = started + seconds
    while (remaining := deadline - time.monotonic()) > 0:
        owner.check()
        time.sleep(min(0.05, remaining))
    owner.check()
    return {"requested_seconds": seconds, "elapsed_seconds": time.monotonic() - started,
            "started_unix": started_unix, "finished_unix": time.time(),
            "ownership_before": before, "ownership_after": require_listener(owner.process.pid, port)}
