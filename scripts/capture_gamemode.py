"""Capture vanilla's reply to the F3+F4 serverbound change_gamemode (0x05).

Sends change_gamemode directly (not via /gamemode chat) with modes 1/3/0 and
dumps EVERY clientbound packet the server replies with, so we can see the exact
packet set (player_info / abilities / game_state_change) and confirm 0x05 is
the correct packet id for 26.2.

usage: python scripts/capture_gamemode.py [port]
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25570

SB_TELEPORT_CONFIRM = 0x00
SB_CHANGE_GAMEMODE = 0x05
SB_KEEP_ALIVE = 0x1c
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_PLAYER_LOADED = 0x2c
SB_SETTINGS = 0x0e

CB_KEEP_ALIVE = 0x2c
CB_POSITION = 0x48
CB_CHUNK_BATCH_FINISHED = 0x0b
CB_KICK = 0x20


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


def join(name, uuid_byte):
    s = socket.create_connection((HOST, PORT), timeout=20)
    s.settimeout(20)
    s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
    s.sendall(wp(0x00, ws(name) + bytes([uuid_byte]) * 16))
    pid, data = rp(s)
    assert pid == 0x02, f'{name} login: got {pid:#x}'
    s.sendall(wp(0x03))
    while True:
        pid, data = rp(s)
        if pid == 0x0e:
            s.sendall(wp(0x07, wv(0)))
        elif pid == 0x02:
            print(f'[{name}] config disconnect:', data.decode('utf-8', 'replace'))
            sys.exit(1)
        elif pid == 0x03:
            break
    s.sendall(wp(0x03))
    data = (ws('en_us') + bytes([10]) + wv(0) + bytes([1]) + bytes([0x7f])
            + wv(1) + bytes([0]) + bytes([1]) + wv(0))
    s.sendall(wp(SB_SETTINGS, data))
    print(f'[{name}] in play state')
    return s


def drain_all(s, seconds, label):
    end = time.time() + seconds
    while time.time() < end:
        s.settimeout(max(0.05, end - time.time()))
        try:
            pid, data = rp(s)
        except (socket.timeout, EOFError):
            break
        print(f'  [{label}] <- 0x{pid:02x} len={len(data)} {data[:48].hex()}')
        if pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
        elif pid == CB_POSITION:
            tid, used = parse_varint(data)
            s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
            s.sendall(wp(SB_PLAYER_LOADED))
        elif pid == CB_CHUNK_BATCH_FINISHED:
            s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        elif pid == CB_KICK:
            print(f'  [{label}] KICKED: '
                  + ''.join(chr(c) if 32 <= c < 127 else '.' for c in data))
            return False
    return True


a = join('AlphaProbe', 0xA1)
drain_all(a, 6, 'join')

for mode in (1, 3, 0):
    print(f'\n=== change_gamemode (0x05) mode={mode} ===')
    a.sendall(wp(SB_CHANGE_GAMEMODE, wv(mode)))
    if not drain_all(a, 2.5, f'gm{mode}'):
        break

a.close()
print('\n[done]')
