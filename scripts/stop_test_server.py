"""Stop leftover test servers (bcore.exe and the vanilla java server).

Decodes tasklist output as bytes: the console code page on a localised Windows
is not UTF-8, so text mode raises UnicodeDecodeError.

usage: python scripts/stop_test_server.py [--vanilla]
"""
import subprocess
import sys
import time


def run(args):
    proc = subprocess.run(args, capture_output=True)
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
    """java.exe processes whose command line runs the bundled vanilla server."""
    out, _ = run(['wmic', 'process', 'where', "name='java.exe'",
                  'get', 'ProcessId,CommandLine', '/FORMAT:CSV'])
    pids = []
    for line in out.splitlines():
        if 'server.jar' not in line:
            continue
        tail = line.rstrip().rsplit(',', 1)[-1]
        try:
            pids.append(int(tail))
        except ValueError:
            pass
    return pids


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
