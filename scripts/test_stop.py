"""Verify /stop shuts a BCore server down.

Joins, runs /stop, and checks that the server disconnects the player and stops
accepting new connections. Run this LAST — it stops the server.

usage: python scripts/test_stop.py [port]     (default 25566)
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25566

CB_CHUNK_BATCH_FINISHED = 0x0b
CB_KICK = 0x20
CB_PROFILELESS_CHAT = 0x21
CB_KEEP_ALIVE = 0x2c
CB_POSITION = 0x48

SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_CHAT_COMMAND = 0x07
SB_KEEP_ALIVE = 0x1c
SB_PLAYER_LOADED = 0x2c


def wv(v):
    out = b''
    v &= 0xFFFFFFFF
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            b |= 0x80
        out += bytes([b])
        if not v:
            return out


def ws(s):
    b = s.encode('utf-8')
    return wv(len(b)) + b


def wp(pid, data=b''):
    body = wv(pid) + data
    return wv(len(body)) + body


def parse_varint(b, at=0):
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError('varint')


buf = b''


def read_n(sock, n):
    global buf
    while len(buf) < n:
        chunk = sock.recv(65536)
        if not chunk:
            raise EOFError('closed')
        buf += chunk
    out, buf = buf[:n], buf[n:]
    return out


def rv(sock):
    r = 0
    for i in range(5):
        b = read_n(sock, 1)[0]
        r |= (b & 0x7F) << (7 * i)
        if not (b & 0x80):
            return r
    raise ValueError('varint')


def rp(sock):
    n = rv(sock)
    data = read_n(sock, n)
    pid, used = parse_varint(data)
    return pid, data[used:]


try:
    s = socket.create_connection((HOST, PORT), timeout=20)
except OSError as exc:
    print(f'cannot reach {HOST}:{PORT} ({exc})')
    raise SystemExit(2)
s.settimeout(20)
s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
s.sendall(wp(0x00, ws('StopProbe') + bytes([0xC3]) * 16))
pid, data = rp(s)
assert pid == 0x02, f'login: 0x{pid:02x}'
s.sendall(wp(0x03))
while True:
    pid, data = rp(s)
    if pid == 0x0e:
        s.sendall(wp(0x07, wv(0)))
    elif pid == 0x03:
        break
s.sendall(wp(0x03))
print('[StopProbe] reached play state')

# Drain the join sequence.
deadline = time.time() + 10
while time.time() < deadline:
    s.settimeout(max(0.05, deadline - time.time()))
    try:
        pid, data = rp(s)
    except (socket.timeout, TimeoutError, EOFError, OSError):
        break
    if pid == CB_POSITION:
        tid, used = parse_varint(data)
        s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
        s.sendall(wp(SB_PLAYER_LOADED))
    elif pid == CB_KEEP_ALIVE:
        s.sendall(wp(SB_KEEP_ALIVE, data))
    elif pid == CB_CHUNK_BATCH_FINISHED:
        s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        break

print('sending /stop')
s.sendall(wp(SB_CHAT_COMMAND, ws('stop')))

saw_notice = False
saw_kick = False
closed = False
deadline = time.time() + 12
while time.time() < deadline and not closed:
    s.settimeout(max(0.05, deadline - time.time()))
    try:
        pid, data = rp(s)
    except (EOFError, ConnectionResetError):
        closed = True
        break
    except (socket.timeout, TimeoutError, OSError):
        break
    if pid == CB_PROFILELESS_CHAT and b'shutting down' in data:
        saw_notice = True
        print('  <- shutdown notice (profileless_chat 0x21)')
    elif pid == CB_KICK:
        saw_kick = True
        print(f'  <- kick_disconnect (0x20): {data[:40].hex()}')

fails = []
if not saw_notice:
    fails.append('no shutdown notice broadcast')
if not saw_kick:
    fails.append('no kick_disconnect after /stop')

try:
    s.close()
except OSError:
    pass

# The listener should refuse (or immediately drop) new connections.
time.sleep(1.5)
still_serving = False
try:
    probe = socket.create_connection((HOST, PORT), timeout=3)
    probe.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(1)))
    probe.sendall(wp(0x00))
    probe.settimeout(3)
    reply = probe.recv(64)
    still_serving = len(reply) > 0
    probe.close()
except OSError:
    still_serving = False
if still_serving:
    fails.append('server still answers status requests after /stop')
else:
    print('  server no longer serves new connections')

if fails:
    print('\nFAILED:')
    for f in fails:
        print(f'  - {f}')
    raise SystemExit(1)
print('\n/stop works: notice broadcast, player disconnected, listener closed')
