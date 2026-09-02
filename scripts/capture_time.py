"""Pin update_time (0x71) clock semantics: which clock id carries day time.

Sends /time set night, /time set day, /time set 6000 as an op and records the
clockUpdates array each time. Prints results; writes nothing.

usage: python scripts/capture_time.py [port]
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25570

CB_CHUNK_BATCH_FINISHED = 0x0b
CB_KICK = 0x20
CB_KEEP_ALIVE = 0x2c
CB_POSITION = 0x48
CB_UPDATE_TIME = 0x71
CB_SYSTEM_CHAT = 0x79

SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_KEEP_ALIVE = 0x1c
SB_PLAYER_LOADED = 0x2c
SB_CHAT_COMMAND = 0x07
SB_SETTINGS = 0x0e


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


def decode_time(data):
    age = struct.unpack_from('>q', data, 0)[0]
    i = 8
    cnt, used = parse_varint(data, i)
    i += used
    ups = []
    for _ in range(cnt):
        cid, used = parse_varint(data, i); i += used
        tt, used = parse_varint(data, i); i += used
        pt = struct.unpack_from('>f', data, i)[0]; i += 4
        rate = struct.unpack_from('>f', data, i)[0]; i += 4
        ups.append((cid, tt, pt, rate))
    return age, ups, len(data) - i


s = socket.create_connection((HOST, PORT), timeout=20)
s.settimeout(20)
s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
s.sendall(wp(0x00, ws('AlphaProbe') + bytes([0xA1]) * 16))
pid, data = rp(s)
assert pid == 0x02, f'login: {pid:#x}'
s.sendall(wp(0x03))
while True:
    pid, data = rp(s)
    if pid == 0x0e:
        s.sendall(wp(0x07, wv(0)))
    elif pid == 0x03:
        break
s.sendall(wp(0x03))
s.sendall(wp(SB_SETTINGS, ws('en_us') + bytes([10]) + wv(0) + bytes([1])
                        + bytes([0x7f]) + wv(1) + bytes([0]) + bytes([1]) + wv(0)))
print('[join] play state')


def drain(seconds, label):
    end = time.time() + seconds
    hits = []
    while time.time() < end:
        s.settimeout(max(0.05, end - time.time()))
        try:
            pid, data = rp(s)
        except (socket.timeout, EOFError):
            break
        if pid == CB_UPDATE_TIME:
            age, ups, left = decode_time(data)
            if ups:
                hits.append((age, ups, left, data.hex()))
        elif pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
        elif pid == CB_POSITION:
            tid, used = parse_varint(data)
            s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
            s.sendall(wp(SB_PLAYER_LOADED))
        elif pid == CB_CHUNK_BATCH_FINISHED:
            s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        elif pid == CB_SYSTEM_CHAT:
            pass
        elif pid == CB_KICK:
            print('KICKED')
            return hits
    print(f'  [{label}] update_time packets WITH clockUpdates: {len(hits)}')
    for age, ups, left, hexs in hits[:4]:
        print(f'     age={age} updates={ups} left={left}')
        print(f'     raw={hexs}')
    return hits


drain(6, 'join')
for cmd, expect in (('time set night', 13000), ('time set day', 1000),
                    ('time set 6000', 6000), ('time set midnight', 18000)):
    print(f'\n=== /{cmd}  (expect day time {expect}) ===')
    s.sendall(wp(SB_CHAT_COMMAND, ws(cmd)))
    drain(2.5, cmd)

try:
    s.close()
except OSError:
    pass
