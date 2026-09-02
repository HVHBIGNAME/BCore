"""Stop leftover test servers (bcore.exe and the vanilla java server).

Decodes tasklist output as bytes: the console code page on a localised Windows
is not UTF-8, so text mode raises UnicodeDecodeError.

usage: python scripts/stop_test_server.py [--vanilla] [--port N]

WARNING: without `--port`, every bcore.exe is stopped — including a live server
sharing the machine. Pass `--port 25566` to stop only the test instance.
"""
import subprocess
import sys
import time


def run(args):
    try:
        proc = subprocess.run(args, capture_output=True)
    except (FileNotFoundError, OSError):
        # e.g. `wmic`, removed in recent Windows 11 builds.
        return '', 127
    return (proc.stdout or b'').decode('utf-8', 'replace'), proc.returncode


def pids_for(image):
    out, _ = run(['tasklist', '/FI', f'IMAGENAME eq {image}', '/FO', 'CSV', '/NH'])
    pids = []
    for line in out.splitlines():
        parts = [p.strip('"') for p in line.split('","')]
        if len(parts) >= 2 and parts[0].lower() == image.lower():
            try:
                pids.append(int(parts[1]))
            except ValueError:
                pass
    return pids


def vanilla_pids():
    """java.exe processes whose command line runs the bundled vanilla server.

    `wmic` was removed in recent Windows 11 builds, so PowerShell's CIM query is
    tried first and `wmic` is only a fallback for older hosts.
    """
    pids = []
    out, rc = run([
        'powershell', '-NoProfile', '-Command',
        "Get-CimInstance Win32_Process -Filter \"name='java.exe'\" | "
        "Where-Object { $_.CommandLine -like '*server.jar*' } | "
        "ForEach-Object { $_.ProcessId }",
    ])
    if rc == 0:
        for line in out.splitlines():
            line = line.strip()
            if line.isdigit():
                pids.append(int(line))
    if pids:
        return pids
    # Fallback for hosts that still ship wmic.
    out, _ = run(['wmic', 'process', 'where', "name='java.exe'",
                  'get', 'ProcessId,CommandLine', '/FORMAT:CSV'])
    for line in out.splitlines():
        if 'server.jar' not in line:
            continue
        tail = line.rstrip().rsplit(',', 1)[-1]
        try:
            pids.append(int(tail))
        except ValueError:
            pass
    return pids


def pid_on_port(port):
    """The pid listening on `port`, if any (encoding-safe netstat parse)."""
    out, _ = run(['netstat', '-ano'])
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 5 and 'LISTEN' in parts[3].upper() \
                and parts[1].endswith(f':{port}'):
            try:
                return int(parts[4])
            except ValueError:
                pass
    return None


def stop(pids, label):
    if not pids:
        print(f'no {label} running')
        return True
    for pid in pids:
        _, rc = run(['taskkill', '/PID', str(pid), '/T', '/F'])
        print(f'killed {label} pid {pid} (rc={rc})')
    return False


def main():
    also_vanilla = '--vanilla' in sys.argv or '--all' in sys.argv

    # `--port N` stops ONLY the server listening on that port. Use it when a
    # live server shares the machine with a test one: the bare invocation stops
    # every bcore.exe, which would take the live server down too.
    port = None
    if '--port' in sys.argv:
        at = sys.argv.index('--port')
        if at + 1 < len(sys.argv):
            try:
                port = int(sys.argv[at + 1])
            except ValueError:
                print(f'bad --port value: {sys.argv[at + 1]}')
                return 2
    if port is not None:
        pid = pid_on_port(port)
        if pid is None:
            print(f'nothing listening on port {port}')
            return 0
        stop([pid], f'server on port {port}')
        for _ in range(20):
            time.sleep(0.25)
            if pid_on_port(port) is None:
                print('stopped')
                return 0
        print(f'WARNING: port {port} still has a listener')
        return 1

    stop(pids_for('bcore.exe'), 'bcore.exe')
    if also_vanilla:
        stop(vanilla_pids(), 'vanilla server')
    for _ in range(20):
        time.sleep(0.25)
        left = pids_for('bcore.exe') + (vanilla_pids() if also_vanilla else [])
        if not left:
            print('all stopped')
            return 0
    print('WARNING: still running:', pids_for('bcore.exe'),
          vanilla_pids() if also_vanilla else [])
    return 1


if __name__ == '__main__':
    sys.exit(main())
