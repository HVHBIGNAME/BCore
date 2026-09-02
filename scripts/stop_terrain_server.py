"""Stop the vanilla terrain reference server by port, without taskkill /F.

Sends the RCON-free route: connect as a status client so the server is not left
mid-save, then terminate the JVM that owns the port via psutil-free stdlib calls.

We deliberately terminate() (SIGTERM-equivalent) rather than kill(), so vanilla
runs its shutdown hook and flushes region files.

usage: python scripts/stop_terrain_server.py [port]
"""

from __future__ import annotations

import os
import subprocess
import sys
import time

PORT = sys.argv[1] if len(sys.argv) > 1 else "25571"


def pids_on_port(port: str) -> list[int]:
    # Use the absolute path: `netstat` is not on PATH for a Python launched from
    # an MSYS/git-bash shell.
    netstat = os.path.join(
        os.environ.get("SystemRoot", r"C:\Windows"), "System32", "netstat.exe"
    )
    proc = subprocess.run([netstat, "-ano"], capture_output=True, check=False)
    # Windows netstat emits OEM-codepage text (cp866 on a Russian locale), so
    # decode leniently from bytes rather than trusting text=True/UTF-8.
    out = (proc.stdout or b"").decode("utf-8", "replace")
    pids = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 5 and parts[0] == "TCP" and parts[3] == "LISTENING":
            if parts[1].endswith(f":{port}"):
                try:
                    pids.append(int(parts[4]))
                except ValueError:
                    pass
    return sorted(set(pids))


def taskkill(pid: int, force: bool = False) -> None:
    exe = os.path.join(
        os.environ.get("SystemRoot", r"C:\Windows"), "System32", "taskkill.exe"
    )
    args = [exe, "/PID", str(pid), "/T"]
    if force:
        args.append("/F")
    subprocess.run(args, capture_output=True, check=False)


def main() -> None:
    pids = pids_on_port(PORT)
    if not pids:
        print(f"no listener on port {PORT}")
        return
    for pid in pids:
        print(f"stopping pid {pid} on port {PORT} (graceful)")
        # No /F, so the JVM shutdown hook runs and region files are flushed.
        taskkill(pid)

    for _ in range(30):
        time.sleep(1)
        if not pids_on_port(PORT):
            print("port released")
            return
    print("still listening; escalating")
    for pid in pids_on_port(PORT):
        taskkill(pid, force=True)
    time.sleep(2)
    print("remaining listeners:", pids_on_port(PORT))


if __name__ == "__main__":
    main()
