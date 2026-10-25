use std::io::{self, Cursor, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use bcore_core::{varint::encode_varint, BlockPos};
use bcore_protocol::blocks::packets::*;
use bcore_protocol::packet::{
    read_frame, read_string, read_varint, write_packet, write_string, PacketError,
};
use bcore_protocol::world::{
    CB_CHUNK_BATCH_FINISHED, CB_MAP_CHUNK, CB_POSITION, SB_PLAYER_LOADED, SB_POSITION,
};

#[derive(Debug, Clone)]
pub struct Packet {
    pub id: i32,
    pub body: Vec<u8>,
}

pub struct Client {
    pub stream: TcpStream,
    pub seen: Vec<Packet>,
    pub session: [u8; 16],
    pub confirm_teleports: bool,
    pending: Vec<u8>,
}

impl Client {
    pub fn join(address: SocketAddr, name: &str, uuid: u8, loaded: bool) -> Self {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut handshake = Vec::new();
        encode_varint(776, &mut handshake);
        write_string("127.0.0.1", &mut handshake);
        handshake.extend_from_slice(&address.port().to_be_bytes());
        encode_varint(2, &mut handshake);
        send(&mut stream, 0, &handshake);
        let mut login = Vec::new();
        write_string(name, &mut login);
        login.extend_from_slice(&[uuid; 16]);
        send(&mut stream, 0, &login);
        let (id, data) = read_frame(&mut stream).unwrap();
        assert_eq!(id, 2);
        let mut cursor = Cursor::new(data);
        let mut identity = [0; 16];
        cursor.read_exact(&mut identity).unwrap();
        assert_eq!(identity, [uuid; 16]);
        assert_eq!(read_string(&mut cursor, 16).unwrap(), name);
        assert_eq!(read_varint(&mut cursor).unwrap(), 0);
        let mut session = [0; 16];
        cursor.read_exact(&mut session).unwrap();
        assert_eq!(cursor.position(), cursor.get_ref().len() as u64);
        send(&mut stream, 3, &[]);
        loop {
            match read_frame(&mut stream).unwrap().0 {
                14 => send(&mut stream, 7, &[0]),
                3 => break,
                _ => {}
            }
        }
        send(&mut stream, 3, &[]);
        let mut client = Self {
            stream,
            seen: Vec::new(),
            session,
            confirm_teleports: true,
            pending: Vec::new(),
        };
        assert!(client.wait(|packets| packets
            .iter()
            .filter(|packet| packet.id == CB_MAP_CHUNK)
            .count()
            == 9
            && packets
                .last()
                .is_some_and(|packet| packet.id == CB_CHUNK_BATCH_FINISHED)));
        if loaded {
            client.send(SB_PLAYER_LOADED, &[]);
        }
        client
    }

    fn next(&mut self, deadline: Instant) -> Result<Option<Packet>, PacketError> {
        let mut bytes = [0; 16 * 1024];
        loop {
            let mut cursor = Cursor::new(self.pending.as_slice());
            match read_frame(&mut cursor) {
                Ok((id, body)) => {
                    let consumed = cursor.position() as usize;
                    self.pending.drain(..consumed);
                    return Ok(Some(Packet { id, body }));
                }
                Err(PacketError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {}
                Err(error) => return Err(error),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            self.stream.set_read_timeout(Some(remaining))?;
            match self.stream.read(&mut bytes) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                Ok(length) => self.pending.extend_from_slice(&bytes[..length]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn collect(&mut self, duration: Duration, done: impl Fn(&[Packet]) -> bool) -> bool {
        let deadline = Instant::now() + duration;
        if done(&self.seen) {
            return true;
        }
        while let Some(packet) = self.next(deadline).expect("clientbound packet") {
            if packet.id == CB_POSITION && self.confirm_teleports {
                let (_, length) = bcore_core::varint::decode_varint(&packet.body).unwrap();
                self.send(0, &packet.body[..length]);
            }
            if packet.id == 44 {
                self.send(28, &packet.body);
            }
            if packet.id == CB_CHUNK_BATCH_FINISHED {
                self.send(11, &4.0f32.to_be_bytes());
            }
            self.seen.push(packet);
            if done(&self.seen) {
                return true;
            }
        }
        done(&self.seen)
    }

    pub fn wait(&mut self, done: impl Fn(&[Packet]) -> bool) -> bool {
        self.collect(Duration::from_secs(5), done)
    }

    pub fn clear(&mut self) {
        self.seen.clear();
    }

    pub fn send(&mut self, id: i32, payload: &[u8]) {
        send(&mut self.stream, id, payload);
    }

    pub fn mode(&mut self, mode: i32) {
        self.clear();
        let mut payload = Vec::new();
        encode_varint(mode, &mut payload);
        self.send(bcore_protocol::chat::SB_CHANGE_GAMEMODE, &payload);
        assert!(self.wait(|packets| packets.iter().any(|packet| packet.id
            == bcore_protocol::gameplay::CB_GAME_STATE_CHANGE
            && packet.body == [vec![3], (mode as f32).to_be_bytes().to_vec()].concat())));
    }

    pub fn move_to(&mut self, x: f64, y: f64, z: f64) {
        let mut payload = Vec::new();
        for coordinate in [x, y, z] {
            payload.extend_from_slice(&coordinate.to_be_bytes());
        }
        payload.push(1);
        self.send(SB_POSITION, &payload);
    }

    pub fn hotbar(&mut self, slot: u8, item: &str) {
        self.clear();
        let item = bcore_registry::registry_id("ITEM", item).unwrap();
        let mut payload = i16::from(36 + slot).to_be_bytes().to_vec();
        encode_varint(64, &mut payload);
        encode_varint(item as i32, &mut payload);
        payload.extend_from_slice(&[0, 0]);
        self.send(SB_SET_CREATIVE_SLOT, &payload);
        assert!(self.wait(|packets| packets
            .iter()
            .any(|packet| packet.id == CB_PLAYER_INVENTORY)));
    }

    pub fn carried(&mut self, slot: u8) {
        self.send(SB_SET_CARRIED_ITEM, &i16::from(slot).to_be_bytes());
    }

    pub fn action(&mut self, action: u8, pos: BlockPos, sequence: i32) {
        let mut payload = vec![action];
        payload.extend_from_slice(&packed(pos));
        payload.push(1);
        encode_varint(sequence, &mut payload);
        self.send(SB_PLAYER_ACTION, &payload);
    }

    pub fn place(
        &mut self,
        pos: BlockPos,
        face: u8,
        hand: u8,
        offsets: [f32; 3],
        world_border: bool,
        sequence: i32,
    ) {
        let mut payload = Vec::new();
        encode_varint(i32::from(hand), &mut payload);
        payload.extend_from_slice(&packed(pos));
        encode_varint(i32::from(face), &mut payload);
        for offset in offsets {
            payload.extend_from_slice(&offset.to_be_bytes());
        }
        payload.extend_from_slice(&[0, u8::from(world_border)]);
        encode_varint(sequence, &mut payload);
        self.send(SB_USE_ITEM_ON, &payload);
    }

    pub fn ack(&mut self, sequence: i32) {
        assert!(self.wait(
            |packets| packets.iter().any(|packet| packet.id == CB_BLOCK_ACK
                && read_varint(&mut Cursor::new(&packet.body)).unwrap() >= sequence)
        ));
    }

    pub fn expect_closed(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.next(deadline) {
                Ok(Some(packet)) => self.seen.push(packet),
                Err(PacketError::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    break
                }
                result => panic!("malformed action failed to close: {result:?}"),
            }
        }
    }

    pub fn command(&mut self, text: &str) {
        let mut payload = Vec::new();
        write_string(text, &mut payload);
        self.send(bcore_protocol::chat::SB_CHAT_COMMAND, &payload);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

fn send(stream: &mut TcpStream, id: i32, payload: &[u8]) {
    let mut frame = Vec::new();
    write_packet(&mut frame, id, payload);
    stream.write_all(&frame).unwrap();
}

pub fn packed(pos: BlockPos) -> [u8; 8] {
    (((pos.x as u64 & 0x3ff_ffff) << 38)
        | ((pos.z as u64 & 0x3ff_ffff) << 12)
        | (pos.y as u64 & 0xfff))
        .to_be_bytes()
}

pub fn changed(packets: &[Packet], pos: BlockPos, state: u32) -> bool {
    packets.iter().any(|packet| {
        packet.id == CB_BLOCK_UPDATE
            && packet.body.starts_with(&packed(pos))
            && read_varint(&mut Cursor::new(&packet.body[8..])).unwrap() == state as i32
    })
}
