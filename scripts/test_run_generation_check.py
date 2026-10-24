"""Windows queue behavior; isolated runner copies and evidence stay under target/."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


PROBE = """
import json
import os
import sys
print(json.dumps(dict(args=sys.argv[1:], cwd=os.getcwd(), env={
    key: os.environ[key]
    for key in ('CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS', 'RAYON_NUM_THREADS')
})), flush=True)
print('child stderr evidence', file=sys.stderr, flush=True)
"""

GATED_CHILD = """
import json
from pathlib import Path
import sys
import time
root, name = Path(sys.argv[1]), sys.argv[2]
active = root / 'active child'
active.touch(exist_ok=False)
try:
    print(json.dumps(dict(start=time.monotonic_ns())), flush=True)
    (root / (name + '.started')).touch()
    if name == 'holder':
        deadline = time.monotonic() + 15
        while not (root / 'release').exists():
            if time.monotonic() >= deadline:
                raise TimeoutError('test did not release the child')
            time.sleep(0.02)
    else:
        time.sleep(0.05)
    print(json.dumps(dict(end=time.monotonic_ns())), flush=True)
finally:
    active.unlink()
"""


@unittest.skipUnless(sys.platform == "win32", "requires the Windows msvcrt queue")
class GenerationCheckTests(unittest.TestCase):
    def setUp(self):
        source = Path(__file__).resolve().with_name("run_generation_check.py")
        target = source.parents[1] / "target"
        target.mkdir(exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="generation-check-test ", dir=target))
        print(f"\nQueue test evidence: {self.root}", flush=True)
        (self.root / ".git").mkdir()
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.runner = scripts / source.name
        shutil.copyfile(source, self.runner)
        self.probe = scripts / "child probe.py"
        self.probe.write_text(PROBE, encoding="utf-8")
        self.gated = scripts / "gated child.py"
        self.gated.write_text(GATED_CHILD, encoding="utf-8")
        self.queue = self.root / "target/generation-checks"
        self.processes = []
        self.background_jobs = []
        self.addCleanup(self.finish_children)

    def finish_children(self):
        (self.root / "release").touch()
        for process in self.processes:
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
                self.fail(f"Queue runner did not exit; evidence: {self.root}")
        for receipt in self.background_jobs:
            self.wait_runner_log(receipt)

    def start(self, label, command, *, background=False, workdir=None):
        argv = [sys.executable, str(self.runner)]
        if background:
            argv.append("--background")
        if workdir is not None:
            argv.extend(["--workdir", str(workdir)])
        argv.extend([label, "--", *command])
        output = self.root / (label + ".submission.log")
        env = dict(os.environ, CARGO_TARGET_DIR="overridden-by-runner",
                   CARGO_BUILD_JOBS="99", RAYON_NUM_THREADS="99")
        with output.open("xb") as log:
            process = subprocess.Popen(argv, cwd=self.runner.parent, env=env,
                                       stdin=subprocess.DEVNULL, stdout=log,
                                       stderr=subprocess.STDOUT)
        self.processes.append(process)
        return process, output

    def collect(self, submission, returncode=0, timeout=30):
        process, output = submission
        process.wait(timeout=timeout)
        text = output.read_text(encoding="utf-8", errors="replace")
        self.assertEqual(process.returncode, returncode, text)
        result = json.loads(text)
        if "pid" in result:
            self.background_jobs.append(result)
        return result

    def wait_for(self, condition, description):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            result = condition()
            if result:
                return result
            time.sleep(0.01)
        self.fail(f"Timed out waiting for {description}; evidence: {self.root}")

    def read_status(self, path):
        # Replacement can also briefly deny readers access on Windows.
        for attempt in range(100):
            try:
                text = path.read_text(encoding="utf-8")
                break
            except PermissionError:
                if attempt == 99:
                    raise
                time.sleep(0.01)
        try:
            return json.loads(text)
        except json.JSONDecodeError as error:
            (self.root / "invalid-status-read.txt").write_text(text, encoding="utf-8")
            self.fail(f"Unreadable status {path}: {error}")

    def records(self):
        return [self.read_status(path) for path in self.queue.glob("*.json")]

    def wait_runner_log(self, receipt):
        path = self.queue / (receipt["job"] + ".runner.log")
        self.wait_for(lambda: path.exists() and path.read_text(encoding="utf-8").endswith("}\n"),
                      "background runner output")
        return json.loads(path.read_text(encoding="utf-8"))

    def complete_background(self, receipt):
        record = self.wait_runner_log(receipt)
        self.assertEqual(self.read_status(Path(receipt["status"])), record)
        self.assertEqual(record["runner_pid"], receipt["pid"])
        self.assertEqual(Path(receipt["log"]), self.queue / (receipt["job"] + ".log"))
        return record

    def assert_finished(self, record, exit_code=0):
        self.assertEqual(record["state"], "finished", record)
        self.assertEqual(record["exit_code"], exit_code)
        self.assertLessEqual(record["started"], record["execution_started"])
        self.assertLessEqual(record["execution_started"], record["finished"])
        self.assertEqual(self.read_status(self.queue / (record["job"] + ".json")), record)

    def check_probe(self, record, arguments, workdir):
        self.assert_finished(record)
        self.assertEqual(Path(record["workdir"]), workdir)
        self.assertEqual(Path(record["cargo_target_dir"]), self.root / "target/build")
        lines = (self.queue / (record["job"] + ".log")).read_text(encoding="utf-8").splitlines()
        payload = json.loads(lines[0])
        self.assertEqual(payload["args"], arguments)
        self.assertEqual(Path(payload["cwd"]), workdir)
        self.assertEqual(payload["env"], dict(CARGO_TARGET_DIR=str(self.root / "target/build"),
                                              CARGO_BUILD_JOBS="1", RAYON_NUM_THREADS="2"))
        self.assertEqual(lines[1:], ["child stderr evidence"])

    def test_foreground_preserves_arguments_environment_and_logs(self):
        arguments = ["two words", 'a "quoted" value', "C:\\path with spaces\\", "", "--background", "--"]
        command = [sys.executable, str(self.probe), *arguments]
        submission = self.start("foreground", command)
        record = self.collect(submission)
        self.assertEqual(record["runner_pid"], submission[0].pid)
        self.assertEqual(record["command"], command)
        self.check_probe(record, arguments, self.root)

    def test_background_workdir_and_arguments(self):
        workdir = self.root / "worktree with spaces"
        workdir.mkdir()
        (workdir / ".git").write_text("gitdir: synthetic-worktree\n", encoding="utf-8")
        arguments = ["two words", 'a "quoted" value', "C:\\path with spaces\\", "", "--background", "--"]
        command = [sys.executable, str(self.probe), *arguments]
        receipt = self.collect(self.start("workdir", command, background=True, workdir=workdir))
        record = self.complete_background(receipt)
        self.assertEqual(record["command"], command)
        self.check_probe(record, arguments, workdir)

    def test_background_returns_while_child_is_running(self):
        command = [sys.executable, str(self.gated), str(self.root), "holder"]
        receipt = self.collect(self.start("background", command, background=True), timeout=10)
        self.wait_for(lambda: (self.root / "holder.started").exists(), "background child start")
        record = self.read_status(Path(receipt["status"]))
        self.assertEqual(record["state"], "running")
        self.assertNotIn("finished", record)
        self.assertTrue((self.root / "active child").exists())
        (self.root / "release").touch()
        self.assert_finished(self.complete_background(receipt))

    def test_concurrent_foreground_and_background_submissions_serialize(self):
        def command(name):
            return [sys.executable, str(self.gated), str(self.root), name]

        holder = self.start("holder", command("holder"))
        self.wait_for(lambda: (self.root / "holder.started").exists(), "lock holder child")
        first = self.start("first", command("first"))
        background = self.start("background", command("background"), background=True)
        last = self.start("last", command("last"))
        receipt = self.collect(background, timeout=10)
        self.wait_for(lambda: len(self.records()) == 4, "all concurrent submissions")
        self.assertEqual(sorted(record["state"] for record in self.records()),
                         ["running", "waiting", "waiting", "waiting"])
        for name in ("first", "background", "last"):
            self.assertFalse((self.root / (name + ".started")).exists())
        (self.root / "release").touch()
        records = [self.collect(item) for item in (holder, first, last)]
        records.append(self.complete_background(receipt))
        intervals = []
        for record in records:
            self.assert_finished(record)
            lines = (self.queue / (record["job"] + ".log")).read_text(encoding="utf-8").splitlines()
            intervals.append((json.loads(lines[0])["start"], json.loads(lines[1])["end"]))
        intervals.sort()
        for previous, following in zip(intervals, intervals[1:]):
            self.assertLessEqual(previous[1], following[0], intervals)

    def check_failure_and_followup(self, command, background, exit_code=None):
        label = "failure-background" if background else "failure-foreground"
        submission = self.start(label, command, background=background)
        result = self.collect(submission, returncode=0 if background else 1)
        record = self.complete_background(result) if background else result
        self.assertEqual(record["command"], command)
        if exit_code is None:
            self.assertEqual(record["state"], "failed")
            self.assertTrue(record["error"])
            self.assertNotIn("exit_code", record)
            self.assertLessEqual(record["execution_started"], record["finished"])
        else:
            self.assert_finished(record, exit_code)
        status = self.queue / (record["job"] + ".json")
        log = self.queue / (record["job"] + ".log")
        evidence = status.read_bytes(), log.read_bytes()
        if exit_code is not None:
            self.assertEqual(log.read_text(encoding="utf-8").splitlines(),
                             ["failure stdout evidence", "failure stderr evidence"])
        followup = self.collect(self.start(label + "-followup", [sys.executable, str(self.probe)]))
        self.check_probe(followup, [], self.root)
        self.assertEqual((status.read_bytes(), log.read_bytes()), evidence)

    def test_nonzero_child_exit_retains_evidence_and_releases_lock(self):
        code = ("import sys; print('failure stdout evidence', flush=True); "
                "print('failure stderr evidence', file=sys.stderr, flush=True); sys.exit(23)")
        for background in (False, True):
            with self.subTest(background=background):
                self.check_failure_and_followup([sys.executable, "-c", code], background, exit_code=23)

    def test_failed_command_launch_is_recorded_and_releases_lock(self):
        command = [str(self.root / "missing executable.exe"), "argument with spaces"]
        for background in (False, True):
            with self.subTest(background=background):
                self.check_failure_and_followup(command, background)

    def test_status_is_readable_during_transitions(self):
        command = [sys.executable, "-c", "import time; time.sleep(0.05)", "x" * 20000]
        submissions = [self.start(f"durable-{index}", command) for index in range(4)]
        deadline = time.monotonic() + 30
        reads = 0
        while any(process.poll() is None for process, _ in submissions):
            reads += len(self.records())
            self.assertLess(time.monotonic(), deadline, f"Queue did not drain: {self.root}")
            time.sleep(0)
        self.assertGreater(reads, 0)
        for submission in submissions:
            self.assert_finished(self.collect(submission))

    def test_status_publication_survives_a_concurrent_reader(self):
        command = [sys.executable, str(self.gated), str(self.root), "holder"]
        submission = self.start("reader", command)
        self.wait_for(lambda: (self.root / "holder.started").exists(), "child holding the queue")
        status = next(self.queue.glob("*.json"))
        with status.open(encoding="utf-8") as reader:
            self.assertEqual(json.load(reader)["state"], "running")
            (self.root / "release").touch()
            self.wait_for(lambda: not (self.root / "active child").exists(), "child completion")
            time.sleep(0.15)
        self.assert_finished(self.collect(submission))


if __name__ == "__main__":
    unittest.main()
