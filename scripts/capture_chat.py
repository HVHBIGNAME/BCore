"""Capture vanilla 26.2 chat/command clientbound packets.

Joins the live vanilla flat server with TWO clients, then:
  * client A sends a plain chat message  -> expect player_chat (0x41) to B
  * client A sends /help                 -> expect system_chat (0x79) to A
  * client A sends /list, /me hello      -> more chat packets
Every clientbound chat-ish packet (0x41 player_chat, 0x79 system_chat,
0x21 profileless_chat, 0x10 declare_commands) is dumped to
target/vanilla_chat.bin in the usual [count][(pid,len,bytes)...] format.

usage: python scripts/capture_chat.py [port]
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25570
OUT = 'target/vanilla_chat.bin'

CB_DECLARE_COMMANDS = 0x10
CB_PROFILELESS_CHAT = 0x21
CB_KICK = 0x20
CB_KEEP_ALIVE = 0x2c
CB_MAP_CHUNK = 0x2d
CB_PLAYER_CHAT = 0x41
CB_POSITION = 0x48
CB_CHUNK_BATCH_FINISHED = 0x0b
CB_UPDATE_HEALTH = 0x68
CB_UPDATE_TIME = 0x71
CB_ABILITIES = 0x40
CB_SYSTEM_CHAT = 0x79

SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_KEEP_ALIVE = 0x1c
SB_PLAYER_LOADED = 0x2c
SB_CHAT_COMMAND = 0x07
SB_CHAT_MESSAGE = 0x09
SB_SETTINGS = 0x0e

CAPTURE_IDS = {CB_PLAYER_CHAT, CB_SYSTEM_CHAT, CB_PROFILELESS_CHAT,
               CB_DECLARE_COMMANDS, CB_UPDATE_HEALTH, CB_UPDATE_TIME,
               CB_ABILITIES}


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


captured = []


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
    print(f'[{name}] in play state')
    return s


def settings(s):
    """Serverbound settings: locale, viewDistance, chatFlags, chatColors,
    skinParts, mainHand, enableTextFiltering, enableServerListing, particles."""
    data = (ws('en_us') + bytes([10]) + wv(0) + bytes([1]) + bytes([0x7f])
            + wv(1) + bytes([0]) + bytes([1]) + wv(0))
    s.sendall(wp(SB_SETTINGS, data))


def drain(s, seconds, label, collect=True):
    end = time.time() + seconds
    while time.time() < end:
        s.settimeout(max(0.05, end - time.time()))
        try:
            pid, data = rp(s)
        except (socket.timeout, EOFError):
            break
        if pid in CAPTURE_IDS and collect:
            captured.append((pid, data))
            print(f'  [{label}] <- 0x{pid:02x} len={len(data)} {data[:64].hex()}')
        elif pid == CB_KEEP_ALIVE:
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
settings(a)
drain(a, 6, 'A')
b = join('BetaProbe', 0xB2)
settings(b)
drain(b, 6, 'B')
drain(a, 2, 'A')

# chat_message: message, timestamp i64, salt i64, signature option(none),
# offset varint, acknowledged buffer[3], checksum u8
def chat_message(text):
    ts = int(time.time() * 1000)
    return (ws(text) + struct.pack('>q', ts) + struct.pack('>q', 0)
            + bytes([0x00]) + wv(0) + bytes(3) + bytes([0x00]))


print('\n=== A sends plain chat "hello from alpha" ===')
a.sendall(wp(SB_CHAT_MESSAGE, chat_message('hello from alpha')))
drain(a, 3, 'A')
drain(b, 3, 'B')

print('\n=== A sends /help ===')
a.sendall(wp(SB_CHAT_COMMAND, ws('help')))
drain(a, 3, 'A')

print('\n=== A sends /list ===')
a.sendall(wp(SB_CHAT_COMMAND, ws('list')))
drain(a, 3, 'A')

print('\n=== A sends /me waves ===')
a.sendall(wp(SB_CHAT_COMMAND, ws('me waves')))
drain(a, 3, 'A')
drain(b, 3, 'B')

print('\n=== A sends /say broadcast test (may need op) ===')
a.sendall(wp(SB_CHAT_COMMAND, ws('say broadcast test')))
drain(a, 3, 'A')
drain(b, 3, 'B')

with open(OUT, 'wb') as f:
    f.write(struct.pack('>I', len(captured)))
    for pid, data in captured:
        f.write(struct.pack('>i', pid))
        f.write(struct.pack('>I', len(data)))
        f.write(data)
print(f'\n[out] wrote {len(captured)} packets to {OUT}')
for s in (a, b):
    try:
        s.close()
    except OSError:
        pass
