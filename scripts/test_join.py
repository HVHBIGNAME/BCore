"""BCore join + movement/chunk-streaming integration test (protocol 776).

Joins the server, verifies the play handshake and the initial chunk batch, then
walks the player into neighbouring chunks and asserts that the server streams
the newly visible chunks (and unloads the ones left behind).

usage: python scripts/test_join.py [port]
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25571

# --- serverbound ids ---
SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_KEEP_ALIVE = 0x1c
SB_POSITION = 0x1e
SB_POSITION_LOOK = 0x1f
SB_PLAYER_LOADED = 0x2c
# --- clientbound ids ---
CB_CHUNK_BATCH_FINISHED = 0x0b
CB_CHUNK_BATCH_START = 0x0c
CB_KICK = 0x20
CB_UNLOAD_CHUNK = 0x25
CB_KEEP_ALIVE = 0x2c
CB_MAP_CHUNK = 0x2d
CB_LOGIN = 0x31
CB_PLAYER_INFO = 0x46
CB_POSITION = 0x48
CB_UPDATE_VIEW_POSITION = 0x5e
CB_SPAWN_POSITION = 0x61

VIEW_DISTANCE = 20


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
            raise EOFError('connection closed')
        d += c
    return d


def rv(sock):
    r = 0
    for i in range(5):
        b = read_n(sock, 1)[0]
        r |= (b & 0x7F) << (7 * i)
        if not (b & 0x80):
            return r
    raise ValueError('varint too long')


def parse_varint(b, at=0):
    r = 0
    for j in range(5):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError('varint too long')


def zigzag_unsigned_to_signed(v):
    return v - (1 << 32) if v >= (1 << 31) else v


def rp(sock):
    n = rv(sock)
    data = read_n(sock, n)
    pid, used = parse_varint(data)
    return pid, data[used:]


def chunk_of(world):
    return int(world // 16)


def describe_kick(payload):
    text = ''.join(chr(c) if 32 <= c < 127 else '.' for c in payload)
    return text.strip('.')


failures = []


def check(ok, label):
    print(('  [ok] ' if ok else '  [FAIL] ') + label)
    if not ok:
        failures.append(label)
    return ok


# ---------------------------------------------------------------- handshake
s = socket.create_connection((HOST, PORT), timeout=15)
s.settimeout(15)
s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
s.sendall(wp(0x00, ws('TestPlayer') + bytes(range(16))))

pid, data = rp(s)
assert pid == 0x02, f'login: expected success(0x02), got {pid:#x}'
print('[ok] login success')
s.sendall(wp(0x03))

registry_count = 0
while True:
    pid, data = rp(s)
    if pid == 0x07:
        registry_count += 1
    elif pid == 0x0e:
        s.sendall(wp(0x07, wv(0)))
    elif pid == 0x00:
        s.sendall(wp(0x01, data + wv(0)))
    elif pid == 0x02:
        print('[FAIL] config disconnect:', data.decode('utf-8', 'replace'))
        sys.exit(1)
    elif pid == 0x03:
        break
print(f'[ok] config: registry_data x{registry_count}, finish received')
assert registry_count >= 20, f'expected ~29 registry packets, got {registry_count}'
s.sendall(wp(0x03))

# ---------------------------------------------------------------- play / join
seen = set()
chunks = {}          # (x, z) -> times received
unloaded = []
batches = []         # announced batch sizes
view_centers = []
spawn = None
teleport_id = None
kick = None
in_batch = 0

s.settimeout(4)
deadline = time.time() + 12
while time.time() < deadline:
    try:
        pid, data = rp(s)
    except socket.timeout:
        break
    except EOFError:
        break
    seen.add(pid)
    if pid == CB_POSITION:
        tid, used = parse_varint(data)
        teleport_id = tid
        spawn = struct.unpack('>ddd', data[used:used + 24])
        s.sendall(wp(SB_TELEPORT_CONFIRM, wv(tid)))
    elif pid == CB_KEEP_ALIVE:
        s.sendall(wp(SB_KEEP_ALIVE, data))
    elif pid == CB_MAP_CHUNK:
        x, z = struct.unpack('>ii', data[:8])
        chunks[(x, z)] = chunks.get((x, z), 0) + 1
        in_batch += 1
    elif pid == CB_CHUNK_BATCH_START:
        in_batch = 0
    elif pid == CB_CHUNK_BATCH_FINISHED:
        size, _ = parse_varint(data)
        batches.append(size)
        # The client must acknowledge the batch or vanilla-side flow control stalls.
        s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
    elif pid == CB_UNLOAD_CHUNK:
        z, x = struct.unpack('>ii', data[:8])
        unloaded.append((x, z))
    elif pid == CB_UPDATE_VIEW_POSITION:
        cx, used = parse_varint(data)
        cz, _ = parse_varint(data, used)
        view_centers.append((zigzag_unsigned_to_signed(cx), zigzag_unsigned_to_signed(cz)))
    elif pid == CB_KICK:
        kick = describe_kick(data)
        break
    # Stop early once the initial batch has landed.
    if batches and spawn is not None and time.time() > deadline - 9:
        break

print(f'\n[join] {len(seen)} packet types, {len(chunks)} chunks, batches={batches}, spawn={spawn}')
check(CB_LOGIN in seen, 'login (0x31)')
check(CB_POSITION in seen, 'position (0x48)')
check(CB_PLAYER_INFO in seen, 'player_info (0x46)')
check(CB_SPAWN_POSITION in seen, 'spawn_position (0x61)')
check(CB_CHUNK_BATCH_START in seen, 'chunk_batch_start (0x0c)')
check(CB_MAP_CHUNK in seen, 'map_chunk (0x2d)')
check(CB_CHUNK_BATCH_FINISHED in seen, 'chunk_batch_finished (0x0b)')
check(kick is None, f'no kick_disconnect during join (got: {kick})')
check(spawn is not None, 'server sent a spawn position')

if spawn is None or kick is not None:
    print('\nRESULT: FAIL — join did not complete')
    sys.exit(1)

# The initial batch must be the full view square, centred on the spawn chunk.
spawn_chunk = (chunk_of(spawn[0]), chunk_of(spawn[2]))
expected = {(spawn_chunk[0] + dx, spawn_chunk[1] + dz)
            for dx in range(-VIEW_DISTANCE, VIEW_DISTANCE + 1)
            for dz in range(-VIEW_DISTANCE, VIEW_DISTANCE + 1)}
check(set(chunks) == expected,
      f'initial batch is the {2*VIEW_DISTANCE+1}x{2*VIEW_DISTANCE+1} view around {spawn_chunk} '
      f'({len(chunks)} chunks, missing={sorted(expected - set(chunks))}, '
      f'extra={sorted(set(chunks) - expected)})')
check(batches and batches[0] == len(expected),
      f'batch_finished announced {batches[0] if batches else None}, expected {len(expected)}')
check(all(v == 1 for v in chunks.values()), 'no chunk was sent twice')
check(view_centers and view_centers[0] == spawn_chunk,
      f'update_view_position centred on {spawn_chunk} (got {view_centers[:1]})')

# The player must be standing on the flat surface (grass at y=-61, so feet at -60).
check(spawn[1] >= -61.0, f'spawn y={spawn[1]} is at or above the flat surface')

# --------------------------------------------------------- movement streaming
print('\n[move] walking the player across chunk borders')
loaded = set(chunks)
s.settimeout(3)


def walk_to(x, y, z, label):
    """Send a position packet and collect everything the server streams back."""
    global loaded
    target_chunk = (chunk_of(x), chunk_of(z))
    s.sendall(wp(SB_POSITION, struct.pack('>ddd', x, y, z) + bytes([0x01])))

    new_chunks, gone, sizes, centers = {}, [], [], []
    kicked = None
    quiet_until = time.time() + 5.0
    while time.time() < quiet_until:
        try:
            pid, data = rp(s)
        except socket.timeout:
            break
        except EOFError:
            break
        if pid == CB_MAP_CHUNK:
            cx, cz = struct.unpack('>ii', data[:8])
            new_chunks[(cx, cz)] = new_chunks.get((cx, cz), 0) + 1
            quiet_until = time.time() + 1.0
        elif pid == CB_CHUNK_BATCH_FINISHED:
            size, _ = parse_varint(data)
            sizes.append(size)
            s.sendall(wp(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0)))
        elif pid == CB_UNLOAD_CHUNK:
            uz, ux = struct.unpack('>ii', data[:8])
            gone.append((ux, uz))
        elif pid == CB_UPDATE_VIEW_POSITION:
            cx, used = parse_varint(data)
            cz, _ = parse_varint(data, used)
            centers.append((zigzag_unsigned_to_signed(cx), zigzag_unsigned_to_signed(cz)))
        elif pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
        elif pid == CB_KICK:
            kicked = describe_kick(data)
            break

    want = {(target_chunk[0] + dx, target_chunk[1] + dz)
            for dx in range(-VIEW_DISTANCE, VIEW_DISTANCE + 1)
            for dz in range(-VIEW_DISTANCE, VIEW_DISTANCE + 1)}
    expected_new = want - loaded
    expected_gone = loaded - want

    print(f'\n  {label}: -> chunk {target_chunk}')
    print(f'    sent {len(new_chunks)} chunks (expected {len(expected_new)}), '
          f'unloaded {len(gone)} (expected {len(expected_gone)}), batches={sizes}')
    check(kicked is None, f'{label}: no kick (got: {kicked})')
    check(set(new_chunks) == expected_new,
          f'{label}: streamed exactly the newly visible chunks '
          f'(missing={sorted(expected_new - set(new_chunks))}, '
          f'extra={sorted(set(new_chunks) - expected_new)})')
    check(all(v == 1 for v in new_chunks.values()), f'{label}: no duplicate chunks')
    check(set(gone) == expected_gone,
          f'{label}: unloaded exactly the chunks that left the view '
          f'(got {sorted(gone)}, expected {sorted(expected_gone)})')
    if expected_new:
        check(sizes and sizes[0] == len(expected_new),
              f'{label}: batch_finished announced {sizes[0] if sizes else None}, '
              f'expected {len(expected_new)}')
        check(centers and centers[-1] == target_chunk,
              f'{label}: update_view_position -> {target_chunk} (got {centers[-1:]})')
    loaded = want
    return kicked is None


y = spawn[1]
# One chunk east: a 5-chunk column enters, a 5-chunk column leaves.
alive = walk_to(spawn[0] + 16.0, y, spawn[2], 'step east')
# One chunk south, so both axes get exercised.
if alive:
    alive = walk_to(spawn[0] + 16.0, y, spawn[2] + 16.0, 'step south')
# Diagonal step: 9 new chunks.
if alive:
    alive = walk_to(spawn[0] + 32.0, y, spawn[2] + 32.0, 'diagonal step')
# Long teleport: the whole view is replaced in bounded batches.
if alive:
    alive = walk_to(2000.5, y, -3000.5, 'long jump')
# Movement inside the same chunk must not resend anything.
if alive:
    print('\n  no-op move (same chunk):')
    s.sendall(wp(SB_POSITION_LOOK,
                 struct.pack('>ddd', 2001.0, y, -3000.0) + struct.pack('>ff', 90.0, 0.0)
                 + bytes([0x01])))
    quiet = 0
    extra = []
    t_end = time.time() + 2.0
    while time.time() < t_end:
        try:
            pid, data = rp(s)
        except socket.timeout:
            break
        except EOFError:
            break
        if pid == CB_MAP_CHUNK:
            extra.append(struct.unpack('>ii', data[:8]))
        elif pid == CB_KEEP_ALIVE:
            s.sendall(wp(SB_KEEP_ALIVE, data))
    check(not extra, f'staying inside a chunk streams nothing (got {extra})')

# The connection must still be alive after all that movement.
print('\n[alive] verifying the connection survived')
try:
    s.sendall(wp(SB_PLAYER_LOADED))
    s.sendall(wp(SB_POSITION, struct.pack('>ddd', 2001.0, y, -3000.0) + bytes([0x01])))
    check(True, 'server still accepts packets after streaming')
except OSError as e:
    check(False, f'connection broke: {e}')

s.close()

print()
if failures:
    print(f'RESULT: FAIL — {len(failures)} check(s) failed:')
    for f in failures:
        print('  -', f)
    sys.exit(1)
print('RESULT: PASS — player joined, walked across chunk borders, '
      'and the world streamed correctly')
sys.exit(0)
