"""End-to-end chat/command test client for BCore.

Joins a running BCore server with TWO clients, then exercises chat and every
command, decoding what comes back:

  * join extras: declare_commands (0x10), update_health (0x68),
    update_time (0x71), abilities (0x40)
  * plain chat -> player_chat (0x41) to the other player
  * /help, /list, /seed -> system_chat (0x79)
  * /me, /say -> profileless_chat (0x21) to both players
  * /gamemode, /tp, /spawn, /time set -> state packets
  * /kick -> kick_disconnect (0x20) for the target

Exit code 0 means every check passed.

usage: python scripts/test_chat.py [port]           (default 25566)
Start a server on that port first, e.g.:
    cargo run -p bcore -- --port 25566
"""
import socket
import struct
import sys
import time

HOST = '127.0.0.1'
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 25566

CB_CHUNK_BATCH_FINISHED = 0x0b
CB_DECLARE_COMMANDS = 0x10
CB_KICK = 0x20
CB_PROFILELESS_CHAT = 0x21
CB_GAME_STATE_CHANGE = 0x26
CB_KEEP_ALIVE = 0x2c
CB_MAP_CHUNK = 0x2d
CB_ABILITIES = 0x40
CB_PLAYER_CHAT = 0x41
CB_POSITION = 0x48
CB_UPDATE_HEALTH = 0x68
CB_UPDATE_TIME = 0x71
CB_SYSTEM_CHAT = 0x79

SB_TELEPORT_CONFIRM = 0x00
SB_CHUNK_BATCH_RECEIVED = 0x0b
SB_CHAT_COMMAND = 0x07
SB_CHAT_MESSAGE = 0x09
SB_KEEP_ALIVE = 0x1c
SB_PLAYER_LOADED = 0x2c

CHAT_IDS = (CB_SYSTEM_CHAT, CB_PROFILELESS_CHAT, CB_PLAYER_CHAT)

failures = []
checks = 0


def check(ok, label, detail=''):
    global checks
    checks += 1
    if ok:
        print(f'  PASS  {label}')
    else:
        print(f'  FAIL  {label}   {detail}')
        failures.append(label)
    return ok


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
    raise ValueError('varint too long')


def parse_varlong(b, at=0):
    r = 0
    for j in range(10):
        x = b[at + j]
        r |= (x & 0x7F) << (7 * j)
        if not (x & 0x80):
            return r, j + 1
    raise ValueError('varlong too long')


def nbt_strings(data):
    """Every plausible NBT string (u16 length + utf8) inside a payload."""
    out = []
    at = 0
    while at + 2 <= len(data):
        ln = struct.unpack_from('>H', data, at)[0]
        if ln >= 2 and at + 2 + ln <= len(data):
            try:
                text = data[at + 2:at + 2 + ln].decode('utf-8')
            except UnicodeDecodeError:
                pass
            else:
                if all(ord(c) >= 32 for c in text):
                    out.append(text)
        at += 1
    return out


def player_chat_message(data):
    """plainMessage out of a player_chat payload."""
    _, n = parse_varint(data)
    at = n + 16                       # senderUuid
    _, m = parse_varint(data, at)     # index
    at += m
    has_sig = data[at]
    at += 1
    if has_sig:
        at += 256
    ln, k = parse_varint(data, at)
    at += k
    return data[at:at + ln].decode('utf-8', 'replace')


class Client:
    def __init__(self, name, uuid_byte):
        self.name = name
        self.uuid = bytes([uuid_byte]) * 16
        self.seen = []
        self.buf = b''
        self.sock = socket.create_connection((HOST, PORT), timeout=20)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock.settimeout(20)
        self._handshake()

    def _read_n(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError('closed')
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _read_varint_stream(self):
        r = 0
        for i in range(5):
            b = self._read_n(1)[0]
            r |= (b & 0x7F) << (7 * i)
            if not (b & 0x80):
                return r
        raise ValueError('varint')

    def read_packet(self):
        n = self._read_varint_stream()
        data = self._read_n(n)
        pid, used = parse_varint(data)
        return pid, data[used:]

    def send(self, pid, data=b''):
        self.sock.sendall(wp(pid, data))

    def _handshake(self):
        self.send(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2))
        self.send(0x00, ws(self.name) + self.uuid)
        pid, data = self.read_packet()
        assert pid == 0x02, f'{self.name}: login got 0x{pid:02x}'
        self.send(0x03)
        while True:
            pid, data = self.read_packet()
            if pid == 0x0e:
                self.send(0x07, wv(0))
            elif pid == 0x02:
                raise SystemExit(f'{self.name} disconnected in config: '
                                 + data.decode('utf-8', 'replace'))
            elif pid == 0x03:
                break
        self.send(0x03)
        print(f'[{self.name}] reached play state')
        self.pump(8, lambda seen: any(p == CB_CHUNK_BATCH_FINISHED for p, _ in seen))

    def pump(self, budget, done=None):
        """Read until `done(seen)` or the budget expires. Returns done's value."""
        deadline = time.time() + budget
        if done and done(self.seen):
            return True
        while time.time() < deadline:
            self.sock.settimeout(max(0.05, deadline - time.time()))
            try:
                pid, data = self.read_packet()
            except (socket.timeout, TimeoutError, EOFError, OSError):
                break
            self.seen.append((pid, data))
            if pid == CB_KEEP_ALIVE:
                self.send(SB_KEEP_ALIVE, data)
            elif pid == CB_POSITION:
                tid, used = parse_varint(data)
                self.send(SB_TELEPORT_CONFIRM, wv(tid))
                self.send(SB_PLAYER_LOADED)
            elif pid == CB_CHUNK_BATCH_FINISHED:
                self.send(SB_CHUNK_BATCH_RECEIVED, struct.pack('>f', 16.0))
            if done and done(self.seen):
                return True
        return bool(done and done(self.seen))

    def clear(self):
        self.seen = []

    def first(self, pid):
        for p, data in self.seen:
            if p == pid:
                return data
        return None

    def last(self, pid):
        found = None
        for p, data in self.seen:
            if p == pid:
                found = data
        return found

    def count(self, pid):
        return sum(1 for p, _ in self.seen if p == pid)

    def chat_lines(self):
        out = []
        for pid, data in self.seen:
            if pid == CB_PLAYER_CHAT:
                out.append(player_chat_message(data))
            elif pid in (CB_SYSTEM_CHAT, CB_PROFILELESS_CHAT):
                out.append(' | '.join(nbt_strings(data)))
        return out

    def wait_chat(self, needle, budget=6):
        def done(seen):
            for pid, data in seen:
                if pid == CB_PLAYER_CHAT and needle in player_chat_message(data):
                    return True
                if pid in (CB_SYSTEM_CHAT, CB_PROFILELESS_CHAT):
                    if any(needle in s for s in nbt_strings(data)):
                        return True
            return False
        return self.pump(budget, done)

    def chat(self, message):
        data = (ws(message) + struct.pack('>q', int(time.time() * 1000))
                + struct.pack('>q', 0) + bytes([0x00]) + wv(0) + bytes(3)
                + bytes([0x00]))
        self.send(SB_CHAT_MESSAGE, data)

    def command(self, command):
        self.send(SB_CHAT_COMMAND, ws(command))

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


print(f'=== BCore chat/command test against {HOST}:{PORT} ===\n')
try:
    alpha = Client('AlphaProbe', 0xA1)
except (ConnectionRefusedError, OSError) as exc:
    print(f'cannot reach {HOST}:{PORT} ({exc}).')
    print('start a server first:  cargo run -p bcore -- --port %d' % PORT)
    raise SystemExit(2)

# ---------------------------------------------------------------- join extras
print('\n--- join extras ---')
alpha.pump(6, lambda seen: all(any(p == want for p, _ in seen)
                               for want in (CB_DECLARE_COMMANDS, CB_UPDATE_HEALTH,
                                            CB_UPDATE_TIME, CB_ABILITIES)))
tree = alpha.first(CB_DECLARE_COMMANDS)
check(tree is not None, 'declare_commands (0x10) received')
if tree:
    nodes, _ = parse_varint(tree)
    missing = [c for c in (b'help', b'list', b'me', b'say', b'spawn', b'gamemode',
                           b'tp', b'seed', b'time', b'kick', b'stop')
               if c not in tree]
    check(not missing, f'command tree declares all commands ({nodes} nodes)',
          f'missing {missing}')

health = alpha.first(CB_UPDATE_HEALTH)
check(health == bytes.fromhex('41a000001440a00000'),
      'update_health (0x68) = 20 hp / 20 food / 5.0 sat',
      health.hex() if health else 'absent')

abilities = alpha.first(CB_ABILITIES)
check(abilities == bytes.fromhex('003d4ccccd3dcccccd'),
      'abilities (0x40) = survival flags + vanilla speeds',
      abilities.hex() if abilities else 'absent')

tm = alpha.first(CB_UPDATE_TIME)
if check(tm is not None, 'update_time (0x71) received'):
    age = struct.unpack_from('>q', tm, 0)[0]
    cnt, n = parse_varint(tm, 8)
    at = 8 + n
    clocks = []
    for _ in range(cnt):
        cid, n = parse_varint(tm, at); at += n
        ticks, n = parse_varlong(tm, at); at += n
        partial = struct.unpack_from('>f', tm, at)[0]; at += 4
        rate = struct.unpack_from('>f', tm, at)[0]; at += 4
        clocks.append((cid, ticks, partial, rate))
    check(cnt == 2 and [c[0] for c in clocks] == [1, 0] and at == len(tm),
          f'update_time carries both clocks (age={age})', str(clocks))
    check(clocks[1][1] == 1000 and clocks[0][3] == 1.0,
          'day time starts at 1000 with rate 1.0', str(clocks))

# ------------------------------------------------------------------- /help
print('\n--- /help ---')
alpha.clear()
alpha.command('help')
alpha.wait_chat('/stop')
lines = alpha.chat_lines()
missing = [c for c in ('/help', '/list', '/me', '/say', '/spawn', '/gamemode',
                       '/tp', '/seed', '/time set', '/kick', '/stop')
           if not any(l.startswith(c) or f'| {c}' in l or c in l for l in lines)]
check(not missing, f'/help lists every command ({len(lines)} lines)', f'missing {missing}')
check(alpha.count(CB_SYSTEM_CHAT) >= 11, 'help replies on system_chat (0x79)',
      f'got {alpha.count(CB_SYSTEM_CHAT)}')
print('     server said:')
for line in lines[:4]:
    print(f'       {line}')

# ------------------------------------------------------------------- /list
print('\n--- /list (one player) ---')
alpha.clear()
alpha.command('list')
alpha.wait_chat('AlphaProbe')
lines = alpha.chat_lines()
check(any('1 of a max of 20' in l and 'AlphaProbe' in l for l in lines),
      '/list reports the online player', str(lines))
print(f'     server said: {lines[-1] if lines else "(nothing)"}')

# ------------------------------------------------------------------- /seed
print('\n--- /seed ---')
alpha.clear()
alpha.command('seed')
alpha.wait_chat('Seed:')
lines = alpha.chat_lines()
check(any('Seed:' in l for l in lines), '/seed answers with the world seed', str(lines))
print(f'     server said: {lines[-1] if lines else "(nothing)"}')

# ------------------------------------------------------- broadcast with 2 players
print('\n--- broadcast: two players ---')
beta = Client('BetaProbe', 0xB2)
beta.pump(6, lambda seen: any(p == CB_ABILITIES for p, _ in seen))
alpha.clear()
alpha.pump(2)
joined = any('BetaProbe' in l and 'joined' in l for l in alpha.chat_lines())
check(joined, 'Alpha is told that Beta joined', str(alpha.chat_lines()))

alpha.clear()
beta.clear()
alpha.chat('hello from alpha')
got_beta = beta.wait_chat('hello from alpha')
got_self = alpha.wait_chat('hello from alpha')
check(got_beta, 'plain chat reaches the other player', str(beta.chat_lines()))
check(got_self, 'sender sees its own message', str(alpha.chat_lines()))
pc = beta.first(CB_PLAYER_CHAT)
if check(pc is not None, 'broadcast arrives as player_chat (0x41)'):
    _, n = parse_varint(pc)
    _, m = parse_varint(pc, n + 16)
    check(pc[n:n + 16] == alpha.uuid and pc[n + 16 + m] == 0x00,
          'player_chat carries the sender uuid and is unsigned',
          f'uuid={pc[n:n+16].hex()} sig={pc[n+16+m]:#02x}')
    check(player_chat_message(pc) == 'hello from alpha',
          'plainMessage decodes correctly', player_chat_message(pc))

print('\n--- /list (two players) ---')
alpha.clear()
alpha.command('list')
alpha.wait_chat('BetaProbe')
lines = alpha.chat_lines()
check(any('2 of a max of 20' in l and 'AlphaProbe, BetaProbe' in l for l in lines),
      '/list shows both players, sorted', str(lines))
print(f'     server said: {lines[-1] if lines else "(nothing)"}')

# ------------------------------------------------------------- /me and /say
print('\n--- /me and /say ---')
alpha.clear()
beta.clear()
alpha.command('me waves at everyone')
check(beta.wait_chat('waves at everyone'), '/me reaches the other player',
      str(beta.chat_lines()))
emote = beta.first(CB_PROFILELESS_CHAT)
if emote:
    ln = struct.unpack_from('>H', emote, 1)[0]
    check(emote[0] == 0x08 and emote[3 + ln] == 0x02,
          '/me uses profileless_chat with chat_type emote_command (holder 2)',
          emote[:6].hex())

beta.clear()
alpha.command('say server notice')
check(beta.wait_chat('server notice'), '/say broadcasts to everyone',
      str(beta.chat_lines()))
say = beta.first(CB_PROFILELESS_CHAT)
if say:
    ln = struct.unpack_from('>H', say, 1)[0]
    check(say[3 + ln] == 0x05,
          '/say uses chat_type say_command (holder 5)', f'{say[3+ln]:#02x}')

# --------------------------------------------------------------- /gamemode
print('\n--- /gamemode ---')
alpha.clear()
alpha.command('gamemode creative')
alpha.pump(6, lambda seen: any(p == CB_GAME_STATE_CHANGE for p, _ in seen))
ab = alpha.last(CB_ABILITIES)
gs = alpha.first(CB_GAME_STATE_CHANGE)
check(ab is not None and ab[0] == 0x0d, 'creative sends abilities flags 0x0d',
      ab.hex() if ab else 'absent')
check(gs == bytes.fromhex('033f800000'),
      'creative sends game_state_change reason 3 value 1.0',
      gs.hex() if gs else 'absent')

alpha.clear()
alpha.command('gamemode spectator')
alpha.pump(6, lambda seen: any(p == CB_GAME_STATE_CHANGE for p, _ in seen))
ab = alpha.last(CB_ABILITIES)
gs = alpha.first(CB_GAME_STATE_CHANGE)
check(ab is not None and ab[0] == 0x07 and gs == bytes.fromhex('0340400000'),
      'spectator sends flags 0x07 and gamemode 3.0',
      f'{ab.hex() if ab else "-"} / {gs.hex() if gs else "-"}')
alpha.command('gamemode survival')
alpha.pump(2)

# ------------------------------------------------------------- /tp and /spawn
print('\n--- /tp and /spawn ---')
alpha.clear()
alpha.command('tp 300.5 -60 -200.5')
alpha.pump(8, lambda seen: any(p == CB_POSITION for p, _ in seen))
pos = alpha.first(CB_POSITION)
if check(pos is not None, '/tp sends position (0x48)'):
    tid, n = parse_varint(pos)
    x, y, z = struct.unpack_from('>ddd', pos, n)
    check((x, y, z) == (300.5, -60.0, -200.5) and len(pos) == 61,
          f'/tp coordinates are absolute ({x}, {y}, {z}), 61-byte payload',
          f'len={len(pos)}')
    # Chunks follow the position packet, so keep reading for them. The server
    # only sends chunks the client does not already hold: the spawn view
    # (centre chunk (0,-1)) and the destination view (centre (18,-13)) overlap
    # by 3 columns x 9 rows = 27 chunks, so 441 - 27 = 414 are new.
    view_side = 21                      # 2 * view distance (10) + 1
    overlap_x = len(range(8, 11))       # x: [8..28] vs [-10..10]
    overlap_z = len(range(-11, -2))     # z: [-23..-3] vs [-11..9]
    expect = view_side * view_side - overlap_x * overlap_z
    alpha.pump(25, lambda seen: sum(1 for p, _ in seen if p == CB_MAP_CHUNK) >= expect)
    got = alpha.count(CB_MAP_CHUNK)
    check(got == expect,
          f'/tp streams the destination view, skipping the {overlap_x * overlap_z}'
          f' chunks already loaded ({got} chunks)',
          f'expected {expect}')

alpha.clear()
alpha.command('spawn')
alpha.pump(8, lambda seen: any(p == CB_POSITION for p, _ in seen))
pos = alpha.first(CB_POSITION)
if check(pos is not None, '/spawn sends position (0x48)'):
    tid, n = parse_varint(pos)
    x, y, z = struct.unpack_from('>ddd', pos, n)
    check((x, y, z) == (10.5, -60.0, -3.5),
          f'/spawn returns to the world spawn ({x}, {y}, {z})')

# --------------------------------------------------------------- /time set
print('\n--- /time set ---')
for when, want in (('night', 13000), ('day', 1000), ('noon', 6000),
                   ('midnight', 18000)):
    alpha.clear()
    beta.clear()
    alpha.command(f'time set {when}')
    alpha.pump(6, lambda seen: any(p == CB_UPDATE_TIME for p, _ in seen))
    tm = alpha.first(CB_UPDATE_TIME)
    ok = False
    if tm:
        cnt, n = parse_varint(tm, 8)
        cid, m = parse_varint(tm, 8 + n)
        ticks, _ = parse_varlong(tm, 8 + n + m)
        ok = cnt == 1 and cid == 0 and ticks == want
    check(ok, f'/time set {when} -> day-time clock {want}',
          tm.hex() if tm else 'absent')
beta.pump(3)
check(any(p == CB_UPDATE_TIME for p, _ in beta.seen),
      '/time set is broadcast to other players')

# ------------------------------------------------------------- unknown command
print('\n--- unknown command ---')
alpha.clear()
alpha.command('fly')
alpha.wait_chat('Unknown')
err = alpha.first(CB_SYSTEM_CHAT)
check(err is not None and b'red' in err,
      'unknown commands answer in red, like vanilla',
      str(alpha.chat_lines()))
print(f'     server said: {alpha.chat_lines()[-1] if alpha.chat_lines() else "-"}')

# --------------------------------------------------------------------- /kick
print('\n--- /kick ---')
alpha.clear()
beta.clear()
alpha.command('kick BetaProbe')
kicked = beta.pump(8, lambda seen: any(p == CB_KICK for p, _ in seen))
check(kicked, 'the kicked player receives kick_disconnect (0x20)',
      str([hex(p) for p, _ in beta.seen]))
check(alpha.wait_chat('Kicked BetaProbe'), 'the sender is told about the kick',
      str(alpha.chat_lines()))

time.sleep(1.0)
alpha.clear()
alpha.command('list')
alpha.wait_chat('players online')
lines = alpha.chat_lines()
check(any('1 of a max of 20' in l and 'BetaProbe' not in l for l in lines),
      '/list drops the kicked player', str(lines))
print(f'     server said: {lines[-1] if lines else "-"}')

alpha.close()
beta.close()

print(f'\n=== {checks - len(failures)}/{checks} checks passed ===')
if failures:
    print('FAILED:')
    for f in failures:
        print(f'  - {f}')
    raise SystemExit(1)
print('ALL CHECKS PASSED')
