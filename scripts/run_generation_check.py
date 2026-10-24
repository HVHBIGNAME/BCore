"""Serialize heavy local validation on Windows; retain commands, logs and status.

The runner lives outside target/ so clearing build artifacts does not delete it.
Every delegated build, native capture and benchmark on this machine must use the
same queue. Use --workdir for an existing worktree inside the repository.
"""
import argparse
import json
import msvcrt
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
QUEUE = ROOT / "target/generation-checks"


def write_status(status, record):
    pending = status.with_suffix(".json.tmp")
    pending.write_text(json.dumps(record, indent=2), encoding="utf-8")
    # Windows readers can briefly prevent replacement of the status file.
    for attempt in range(100):
        try:
            pending.replace(status)
            return
        except PermissionError:
            if attempt == 99:
                raise
            time.sleep(0.01)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--background", action="store_true")
    parser.add_argument("--workdir", type=Path, default=ROOT)
    parser.add_argument("--job", help=argparse.SUPPRESS)
    parser.add_argument("label")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    workdir = args.workdir.resolve()
    if not command or Path(args.label).name != args.label:
        parser.error("a simple label and command are required")
    if not workdir.is_relative_to(ROOT) or not (workdir / ".git").exists():
        parser.error("workdir must be an existing worktree inside this repository")
    QUEUE.mkdir(parents=True, exist_ok=True)
    job = args.job or f"{args.label}-{time.time_ns()}"
    if Path(job).name != job:
        parser.error("invalid job name")
    status = QUEUE / (job + ".json")
    record = {"job": job, "state": "waiting", "command": command,
              "workdir": str(workdir), "started": time.time(), "runner_pid": os.getpid()}
    write_status(status, record)
    if args.background:
        with (QUEUE / (job + ".runner.log")).open("xb") as log:
            child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                "--job", job, "--workdir", str(workdir), args.label, "--", *command],
                cwd=ROOT, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
        print(json.dumps({"job": job, "pid": child.pid, "status": str(status),
                          "log": str(QUEUE / (job + ".log"))}), flush=True)
        return
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/build"),
               CARGO_BUILD_JOBS="1", RAYON_NUM_THREADS="2")
    record["cargo_target_dir"] = env["CARGO_TARGET_DIR"]
    with (QUEUE / ".heavy-check.lock").open("a+b") as lock:
        if lock.tell() == 0:
            lock.write(b"\0")
            lock.flush()
        while True:
            try:
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                break
            except OSError:
                time.sleep(2)
        try:
            record.update(state="running", execution_started=time.time())
            write_status(status, record)
            with (QUEUE / (job + ".log")).open("xb") as log:
                result = subprocess.run(command, cwd=workdir, env=env, stdout=log,
                                        stderr=subprocess.STDOUT, timeout=7200)
            record.update(state="finished", exit_code=result.returncode)
        except Exception as error:
            record.update(state="failed", error=str(error))
        finally:
            record["finished"] = time.time()
            write_status(status, record)
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
    print(json.dumps(record, indent=2), flush=True)
    if record.get("exit_code", 1):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
