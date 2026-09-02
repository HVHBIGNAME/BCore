import socket, struct, time, sys

HOST, PORT = '127.0.0.1', int(sys.argv[1]) if len(sys.argv) > 1 else 25571

def wv(v):
    out = b''
    while True:
        b = v & 0x7f
        v >>= 7
        if v: b |= 0x80
        out += bytes([b])
        if not v: return out

def rv(sock):
    r = 0
    for i in range(5):
        b = sock.recv(1)
        if not b: raise EOFError('eof')
        b = b[0]
        r |= (b & 0x7f) << (7*i)
        if not (b & 0x80): return r
    raise ValueError

def ws(s):
    b = s.encode('utf-8')
    return wv(len(b)) + b

def read_n(sock, n):
    d = b''
    while len(d) < n:
        c = sock.recv(n - len(d))
        if not c: break
        d += c
    return d

def wp(pid, data=b''):
    body = wv(pid) + data
    return wv(len(body)) + body

def rp(sock):
    n = rv(sock)
    data = read_n(sock, n)
    i = 0; pid = 0
    for j in range(5):
        b = data[j]; i = j+1
        pid |= (b & 0x7f) << (7*j)
        if not (b & 0x80): break
    return pid, data[i:]

def parse_varint(b):
    r = 0
    for j in range(5):
        x = b[j]
        r |= (x & 0x7f) << (7*j)
        if not (x & 0x80): return r, j+1
    return r, 5

s = socket.create_connection((HOST, PORT), timeout=15)
s.settimeout(15)

# handshake -> login
s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
# login start
name = 'TestPlayer'
s.sendall(wp(0x00, ws(name) + bytes(range(16))))

pid, data = rp(s)
assert pid == 0x02, f'login: expected success(0x02) got {pid:#x}'
print('[ok] login success')
s.sendall(wp(0x03))  # login acknowledged

registry_count = 0
saw_finish = False
while not saw_finish:
    pid, data = rp(s)
    if pid == 0x07:
        registry_count += 1
    elif pid == 0x0e:
        s.sendall(wp(0x07, wv(0)))  # known packs response
    elif pid == 0x00:
        s.sendall(wp(0x01, data + wv(0)))
    elif pid == 0x03:
        saw_finish = True
    elif pid == 0x02:
        print('[fail] config disconnect:', data.decode('utf-8','replace'))
        sys.exit(1)
print(f'[ok] config: registry_data x{registry_count}, finish received')
assert registry_count >= 20, f'expected ~29 registry packets, got {registry_count}'
s.sendall(wp(0x03))  # finish configuration ack

# play phase
seen = set()
s.settimeout(3)
start = time.time()
map_chunks = 0
got_keepalive = False
while time.time() - start < 13:
    try:
        pid, data = rp(s)
    except socket.timeout:
        continue
    except Exception:
        break
    seen.add(pid)
    if pid == 0x48:  # position -> teleport confirm
        s.sendall(wp(0x00, data[:parse_varint(data)[1]]))
    elif pid == 0x2c:  # keep_alive -> echo keep_alive (0x1c)
        s.sendall(wp(0x1c, data))
        got_keepalive = True
    elif pid == 0x2d:
        map_chunks += 1

checks = {
    'JoinGame (0x31)': 0x31 in seen,
    'position (0x48)': 0x48 in seen,
    'player_info (0x46)': 0x46 in seen,
    'chunk_batch_start (0x0c)': 0x0c in seen,
    'map_chunk (0x2d)': 0x2d in seen,
    'chunk_batch_finished (0x0b)': 0x0b in seen,
    'spawn_position (0x61)': 0x61 in seen,
}
print(f'[play] seen {len(seen)} packet types, {map_chunks} chunks, keepalive={got_keepalive}')
for k, v in checks.items():
    print(('  [ok] ' if v else '  [MISSING] ') + k)

ok = all(checks.values())
print('\nRESULT:', 'PASS — player reached play state with world chunks' if ok else 'FAIL')
sys.exit(0 if ok else 1)
