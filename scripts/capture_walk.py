"""Cross-validate BCore's native chunk encoder against the live vanilla server.

Connects to a running vanilla 26.2 flat-world server, walks the player across
chunk borders to force fresh `map_chunk` packets, and dumps every distinct chunk
it receives to `target/vanilla_walk_chunks.bin` (same [count][(pid,len,bytes)...]
format as the other captures).

`cargo test -p bcore-protocol --test chunk_parity_vanilla` then compares those
payloads with `flat_chunk_payload`, so parity is checked against chunks the
server generated *during movement*, not just the join batch.

usage: python scripts/capture_walk.py [port]
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25570
OUT = 'target/vanilla_walk_chunks.bin'

CB_CHUNK_BATCH_FINISHED = 0x0b
CB_KICK = 0x20
CB_KEEP_ALIVE = 0x2c
CB_MAP_CHUNK = 0x2d
CB_POSITION = 0x48
SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_KEEP_ALIVE = 0x1c
SB_POSITION = 0x1e
SB_CLIENT_COMMAND = 0x0c
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


def read_n(sock, n):
    d = b''
    while len(d) < n:
        c = sock.recv(n - len(d))
        if not c:
            raise EOFError('closed')
        d += c
    return d


def rv(sock):
    r = 0
    for i in range(5):
        b = read_n(sock, 1)[0]
        r |= (b & 0x7F) << (7 * i)
        if not (b & 0x80):
            return r
    raise ValueError('varint')


def parse_varint(b, at=0):
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError('varint')


def rp(sock):
    n = rv(sock)
    data = read_n(sock, n)
    pid, used = parse_varint(data)
    return pid, data[used:]


s = socket.create_connection((HOST, PORT), timeout=20)
s.settimeout(20)
s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
s.sendall(wp(0x00, ws('BCoreWalker') + bytes(range(16))))

pid, data = rp(s)
assert pid == 0x02, f'login: got {pid:#x}'
print('[login] success')
s.sendall(wp(0x03))

while True:
    pid, data = rp(s)
    if pid == 0x0e:
        s.sendall(wp(0x07, wv(0)))
    elif pid == 0x00:
        s.sendall(wp(0x01, data + wv(0)))
    elif pid == 0x02:
        print('[config] disconnect:', data.decode('utf-8', 'replace'))
        sys.exit(1)
    elif pid == 0x03:
        break
print('[config] done')
s.sendall(wp(0x03))

chunks = {}
spawn = None
kick = None
s.settimeout(4)

# Drain the join sequence first.
deadline = time.time() + 12
while time.time() < deadline:
    try:
        pid, data = rp(s)
    except (socket.timeout, EOFError):
        break
    if pid == CB_MAP_CHUNK:
        x, z = struct.unpack('>ii', data[:8])
        chunks.setdefault((x, z), data)
    elif pid == CB_POSITION:
        tid, used = parse_varint(data)
        spawn = struct.unpack('>ddd', data[used:used + 24])
        s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
        s.sendall(wp(SB_PLAYER_LOADED))
    elif pid == CB_KEEP_ALIVE:
        s.sendall(wp(SB_KEEP_ALIVE, data))
    elif pid == CB_CHUNK_BATCH_FINISHED:
        s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        if spawn is not None and len(chunks) >= 9:
            break
    elif pid == CB_KICK:
        kick = ''.join(chr(c) if 32 <= c < 127 else '.' for c in data)
        break

print(f'[join] spawn={spawn} chunks={len(chunks)} kick={kick}')
if spawn is None:
    print('FAIL: no spawn position')
    sys.exit(1)

# Walk in small steps so vanilla accepts the movement (no flying/teleport kick).
x0, y0, z0 = spawn
print('[walk] stepping outwards to force fresh chunks')
step = 4.0
for i in range(1, 41):
    x = x0 + step * i
    z = z0 + step * (i // 2)
    s.sendall(wp(SB_POSITION, struct.pack('>ddd', x, y0, z) + bytes([0x01])))
    time.sleep(0.05)
    # Drain whatever arrived.
    s.settimeout(0.25)
    while True:
        try:
            pid, data = rp(s)
        except (socket.timeout, EOFError):
            break
        if pid == CB_MAP_CHUNK:
            cx, cz = struct.unpack('>ii', data[:8])
            chunks.setdefault((cx, cz), data)
        elif pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
        elif pid == CB_CHUNK_BATCH_FINISHED:
            s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        elif pid == CB_KICK:
            kick = ''.join(chr(c) if 32 <= c < 127 else '.' for c in data)
            break
    if kick:
        break

print(f'[walk] collected {len(chunks)} distinct chunks; kick={kick}')
sizes = sorted({len(v) for v in chunks.values()})
print(f'[walk] payload sizes seen: {sizes}')

items = sorted(chunks.items())
with open(OUT, 'wb') as f:
    f.write(struct.pack('>I', len(items)))
    for (_, _), data in items:
        f.write(struct.pack('>i', CB_MAP_CHUNK))
        f.write(struct.pack('>I', len(data)))
        f.write(data)
print(f'[out] wrote {len(items)} chunks to {OUT}')
try:
    s.close()
except OSError:
    pass
