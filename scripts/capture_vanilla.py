import socket, struct, sys, time

HOST, PORT = '127.0.0.1', 25570

def wv(v):
    out = b''
    while True:
        b = v & 0x7f
        v >>= 7
        if v:
            b |= 0x80
        out += bytes([b])
        if not v:
            return out

def rv(sock):
    r = 0
    for i in range(5):
        b = sock.recv(1)
        if not b:
            raise EOFError('eof')
        b = b[0]
        r |= (b & 0x7f) << (7 * i)
        if not (b & 0x80):
            return r
    raise ValueError('varint too big')

def ws(s):
    b = s.encode('utf-8')
    return wv(len(b)) + b

def read_n(sock, n):
    data = b''
    while len(data) < n:
        c = sock.recv(n - len(data))
        if not c:
            break
        data += c
    return data

def wp(pid, data=b''):
    body = wv(pid) + data
    return wv(len(body)) + body

def parse_pid(data):
    i = 0
    pid = 0
    for j in range(5):
        b = data[j]
        i = j + 1
        pid |= (b & 0x7f) << (7 * j)
        if not (b & 0x80):
            break
    return pid, data[i:]

def wvlen(b):
    # length in bytes of the leading varint in b
    i = 0
    for j in range(5):
        i = j + 1
        if not (b[j] & 0x80):
            break
    return i

def rp(sock):
    n = rv(sock)
    data = read_n(sock, n)
    pid, rest = parse_pid(data)
    return pid, rest

s = socket.create_connection((HOST, PORT), timeout=20)
s.settimeout(20)

s.sendall(wp(0x00, wv(776) + ws(HOST) + struct.pack('>H', PORT) + wv(2)))
name = 'BCoreProbe'
s.sendall(wp(0x00, ws(name) + bytes(range(16))))

pid, data = rp(s)
assert pid == 0x02, f'login got {pid}'
print('[login] success')

s.sendall(wp(0x03))  # login acknowledged

config_packets = []
while True:
    pid, data = rp(s)
    config_packets.append((pid, data))
    if pid == 0x0e:  # select_known_packs -> respond empty
        s.sendall(wp(0x07, wv(0)))
    elif pid == 0x00:  # cookie_request
        s.sendall(wp(0x01, data + wv(0)))  # echo key + empty payload (approx)
    elif pid == 0x03:  # finish_configuration
        break
    elif pid == 0x02:  # disconnect
        print('[config] DISCONNECT:', data.decode('utf-8', 'replace'))
        sys.exit(1)

print(f'[config] {len(config_packets)} packets saved')

# finish configuration ack -> play state
s.sendall(wp(0x03))

play_packets = []
start = time.time()
s.settimeout(4)
try:
    while len(play_packets) < 40 and time.time() - start < 12:
        pid, data = rp(s)
        play_packets.append((pid, data))
        if pid == 0x2c:  # keep_alive -> pong
            s.sendall(wp(0x2d, data))
        elif pid == 0x48:  # position -> teleport confirm (echo teleportId varint)
            s.sendall(wp(0x00, data[:wvlen(data)]))
        elif pid == 0x0b:  # chunk_batch_finished -> ack (echo batch size)
            s.sendall(wp(0x0b, data))
except Exception as e:
    print('[play] stop:', e)

print(f'[play] {len(play_packets)} packets saved')

def wvlen(b):
    # length in bytes of the leading varint in b
    i = 0
    for j in range(5):
        i = j + 1
        if not (b[j] & 0x80):
            break
    return i

with open('target/config_packets.bin', 'wb') as f:
    f.write(struct.pack('>I', len(config_packets)))
    for pid, data in config_packets:
        f.write(struct.pack('>i', pid))
        f.write(struct.pack('>I', len(data)))
        f.write(data)

with open('target/play_packets.bin', 'wb') as f:
    f.write(struct.pack('>I', len(play_packets)))
    for pid, data in play_packets:
        f.write(struct.pack('>i', pid))
        f.write(struct.pack('>I', len(data)))
        f.write(data)

# summary of play packets
from collections import Counter
names = {0x31:'login',0x48:'position',0x46:'player_info',0x2c:'keep_alive',
         0x2d:'map_chunk',0x0b:'chunk_batch_finished',0x0c:'chunk_batch_start',
         0x61:'spawn_position',0x5f:'update_view_distance',0x30:'update_light',
         0x57:'action_bar',0x71:'update_time',0x40:'abilities',0x86:'tags',
         0x79:'system_chat',0x18:'custom_payload',0x56:'server_data',0x53:'entity_head_rotation',
         0x05:'block_break_animation'}
c = Counter(names.get(pid, f'0x{pid:02x}') for pid, _ in play_packets)
print('[play] packet types:', dict(c))
for pid, data in play_packets[:20]:
    print(f'  {names.get(pid, hex(pid)):24} len={len(data)}')
