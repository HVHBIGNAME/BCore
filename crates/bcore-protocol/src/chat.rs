//! Chat: serverbound message/command parsing and clientbound chat packets.
//!
//! # Packet ids (protocol 776)
//!
//! Every id below comes from `target/protocol_26_1.json` (PrismarineJS
//! minecraft-data) and was confirmed against live captures from the official
//! 26.2 server (`scripts/capture_chat.py`, `scripts/capture_chat_op.py`).
//!
//! | direction | name                | id     |
//! |-----------|---------------------|--------|
//! | S→C       | `system_chat`       | `0x79` |
//! | S→C       | `player_chat`       | `0x41` |
//! | S→C       | `profileless_chat`  | `0x21` |
//! | C→S       | `chat_command`      | `0x07` |
//! | C→S       | `chat_command_signed` | `0x08` |
//! | C→S       | `chat_message`      | `0x09` |
//! | C→S       | `chat_session_update` | `0x0a` |
//! | C→S       | `message_acknowledgement` | `0x06` |
//!
//! # Which clientbound packet BCore sends
//!
//! Vanilla answers a plain chat message with `player_chat` (0x41), which
//! carries a signature slot, a message-chain index and a `chat_type` registry
//! reference. In offline mode there is nothing to sign, and the capture shows
//! vanilla itself sending `signature = none` / `previousMessages = []` in that
//! case, so [`encode_player_chat`] reproduces that unsigned form exactly.
//!
//! `profileless_chat` (0x21) is what vanilla uses for messages with no player
//! session behind them (`/me`, `/say`); [`encode_profileless_chat`] covers it.
//! Server notices and command replies use `system_chat` (0x79).
//!
//! # chat_type registry ids
//!
//! `chat_type` is sent during configuration (`registry_data`) in this order,
//! read straight out of `data/config_packets.bin`:
//!
//! ```text
//! 0 chat  1 emote_command  2 msg_command_incoming  3 msg_command_outgoing
//! 4 say_command  5 team_msg_command_incoming  6 team_msg_command_outgoing
//! ```
//!
//! On the wire the field is a `registryEntryHolder`: a varint of `id + 1`
//! (0 would mean "inline definition follows"), which is why the capture shows
//! `01` for `minecraft:chat` and `05` for `minecraft:say_command`.

use std::io::Cursor;

use bcore_core::varint::encode_varint;

use crate::nbt::{encode_component, encode_text, Component};
use crate::packet::{read_string, read_varint, write_packet, write_string, PacketError};

/// Clientbound `profileless_chat`.
pub const CB_PROFILELESS_CHAT: i32 = 0x21;
/// Clientbound `player_chat`.
pub const CB_PLAYER_CHAT: i32 = 0x41;
/// Clientbound `system_chat`.
pub const CB_SYSTEM_CHAT: i32 = 0x79;

/// Serverbound `message_acknowledgement`.
pub const SB_MESSAGE_ACKNOWLEDGEMENT: i32 = 0x06;
/// Serverbound `chat_command` (unsigned; what a vanilla client sends for `/cmd`).
pub const SB_CHAT_COMMAND: i32 = 0x07;
/// Serverbound `chat_command_signed` (signed arguments; secure chat only).
pub const SB_CHAT_COMMAND_SIGNED: i32 = 0x08;
/// Serverbound `chat_message`.
pub const SB_CHAT_MESSAGE: i32 = 0x09;
/// Serverbound `chat_session_update` (public-key session; ignored offline).
pub const SB_CHAT_SESSION_UPDATE: i32 = 0x0a;

/// `chat_type` registry id for `minecraft:chat` (normal player chat).
pub const CHAT_TYPE_CHAT: i32 = 0;
/// `chat_type` registry id for `minecraft:emote_command` (`/me`).
pub const CHAT_TYPE_EMOTE_COMMAND: i32 = 1;
/// `chat_type` registry id for `minecraft:say_command` (`/say`).
pub const CHAT_TYPE_SAY_COMMAND: i32 = 4;

/// Longest chat message a client may send (vanilla's limit).
pub const MAX_MESSAGE_CHARS: usize = 256;
/// Longest command a client may send.
pub const MAX_COMMAND_CHARS: usize = 32767;

/// What the client sent us on the chat channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatInput {
    /// A plain message (`chat_message`); no leading slash.
    Message(String),
    /// A command (`chat_command`), already stripped of its leading slash.
    Command(String),
}

/// Parse a serverbound chat packet, returning `None` for unrelated ids.
///
/// Malformed payloads yield `None` rather than an error: a bad chat packet must
/// not drop the connection.
pub fn parse_chat_input(packet_id: i32, data: &[u8]) -> Option<ChatInput> {
    match packet_id {
        // `chat_message`: message(string) then timestamp/salt/signature/... —
        // only the leading string matters without secure chat.
        SB_CHAT_MESSAGE => {
            let mut cur = Cursor::new(data);
            let msg = read_string(&mut cur, MAX_MESSAGE_CHARS).ok()?;
            Some(ChatInput::Message(sanitize(&msg)))
        }
        // `chat_command` / `chat_command_signed` both start with command(string).
        SB_CHAT_COMMAND | SB_CHAT_COMMAND_SIGNED => {
            let mut cur = Cursor::new(data);
            let cmd = read_string(&mut cur, MAX_COMMAND_CHARS).ok()?;
            Some(ChatInput::Command(sanitize(&cmd)))
        }
        _ => None,
    }
}

/// Drop control characters (including the section sign used for legacy colour
/// codes) so a client cannot inject formatting or newlines into other players'
/// chat.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '\u{00a7}')
        .collect()
}

/// Encode `system_chat` (0x79): `anonymousNbt content`, `bool isActionBar`.
pub fn encode_system_chat(component: &Component, action_bar: bool) -> Vec<u8> {
    let mut data = encode_component(component);
    data.push(u8::from(action_bar));
    let mut out = Vec::new();
    write_packet(&mut out, CB_SYSTEM_CHAT, &data);
    out
}

/// Encode `system_chat` carrying unstyled literal text in the chat box.
pub fn encode_system_message(text: &str) -> Vec<u8> {
    let mut data = encode_text(text);
    data.push(0x00); // isActionBar = false
    let mut out = Vec::new();
    write_packet(&mut out, CB_SYSTEM_CHAT, &data);
    out
}

/// Encode an unsigned `player_chat` (0x41) for offline mode.
///
/// Field order (`protocol_26_1.json`, verified byte-for-byte against a capture
/// of vanilla relaying "hello from alpha"):
///
/// ```text
/// globalIndex varint | senderUuid 16 | index varint | signature option(none)
/// plainMessage string | timestamp i64 | salt i64 | previousMessages varint(0)
/// unsignedChatContent option(none) | filterType varint(0=PASS_THROUGH)
/// type varint(registry id + 1) | networkName anonymousNbt
/// networkTargetName option(none)
/// ```
pub fn encode_player_chat(
    global_index: i32,
    sender_uuid: &[u8; 16],
    sender_name: &str,
    message: &str,
    timestamp_millis: i64,
) -> Vec<u8> {
    let mut data = Vec::new();
    encode_varint(global_index, &mut data);
    data.extend_from_slice(sender_uuid);
    encode_varint(0, &mut data); // index within the (absent) message chain
    data.push(0x00); // signature: none — offline mode never signs
    write_string(message, &mut data);
    data.extend_from_slice(&timestamp_millis.to_be_bytes());
    data.extend_from_slice(&0i64.to_be_bytes()); // salt
    encode_varint(0, &mut data); // previousMessages: empty
    data.push(0x00); // unsignedChatContent: none (plainMessage is enough)
    encode_varint(0, &mut data); // filterType: PASS_THROUGH
    encode_varint(CHAT_TYPE_CHAT + 1, &mut data); // registryEntryHolder: id + 1
    data.extend_from_slice(&encode_text(sender_name)); // networkName
    data.push(0x00); // networkTargetName: none

    let mut out = Vec::new();
    write_packet(&mut out, CB_PLAYER_CHAT, &data);
    out
}

/// Encode `profileless_chat` (0x21): a message with no chat session behind it.
///
/// ```text
/// message anonymousNbt | type varint(registry id + 1)
/// name anonymousNbt | target option(anonymousNbt)
/// ```
///
/// Used for `/me` (`chat_type` 1) and `/say` (`chat_type` 4), matching vanilla.
pub fn encode_profileless_chat(message: &str, chat_type: i32, sender_name: &str) -> Vec<u8> {
    let mut data = encode_text(message);
    encode_varint(chat_type + 1, &mut data);
    data.extend_from_slice(&encode_text(sender_name));
    data.push(0x00); // target: none
    let mut out = Vec::new();
    write_packet(&mut out, CB_PROFILELESS_CHAT, &data);
    out
}

/// Read the leading string of a serverbound chat packet without interpreting it.
pub fn peek_chat_string(data: &[u8], max_chars: usize) -> Option<String> {
    let mut cur = Cursor::new(data);
    read_string(&mut cur, max_chars).ok()
}

/// Read a `message_acknowledgement` count (0x06); acknowledged offline as a no-op.
pub fn read_message_acknowledgement(data: &[u8]) -> Result<i32, PacketError> {
    let mut cur = Cursor::new(data);
    read_varint(&mut cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcore_core::varint::decode_varint;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    /// Strip the framing (length varint + packet id varint) off an encoded packet.
    fn payload(frame: &[u8]) -> (i32, Vec<u8>) {
        let (len, n) = decode_varint(frame).expect("length");
        assert_eq!(frame.len(), n + len as usize, "frame length must be exact");
        let (id, m) = decode_varint(&frame[n..]).expect("id");
        (id, frame[n + m..].to_vec())
    }

    #[test]
    fn system_chat_matches_the_captured_vanilla_help_line() {
        // Vanilla 26.2 answering `/help`, packet body captured verbatim:
        // content = TAG_String "/me <action>", isActionBar = 0.
        let (id, body) = payload(&encode_system_message("/me <action>"));
        assert_eq!(id, CB_SYSTEM_CHAT);
        assert_eq!(body, hex("08000c2f6d65203c616374696f6e3e00"));
    }

    #[test]
    fn system_chat_action_bar_flag_is_the_trailing_byte() {
        let (_, chat_box) = payload(&encode_system_chat(&Component::text("hi"), false));
        let (_, bar) = payload(&encode_system_chat(&Component::text("hi"), true));
        assert_eq!(*chat_box.last().expect("flag"), 0x00);
        assert_eq!(*bar.last().expect("flag"), 0x01);
        assert_eq!(chat_box[..chat_box.len() - 1], bar[..bar.len() - 1]);
    }

    #[test]
    fn system_chat_carries_styled_components() {
        let (_, body) = payload(&encode_system_chat(
            &Component::colored("nope", "red"),
            false,
        ));
        assert_eq!(body[0], crate::nbt::TAG_COMPOUND);
        assert!(body.windows(3).any(|w| w == b"red"));
        assert_eq!(*body.last().expect("flag"), 0x00);
    }

    #[test]
    fn player_chat_reproduces_the_captured_unsigned_layout() {
        // Captured from vanilla relaying AlphaProbe's "hello from alpha".
        // Only the trailing networkName component differs from the capture:
        // vanilla decorates the name with click/hover events, BCore sends the
        // plain name. Every fixed-width field before it must match exactly.
        let uuid = hex("ac964522d95e3ef0b4524e380e6762bf");
        let uuid: [u8; 16] = uuid.try_into().expect("16 bytes");
        let (id, body) = payload(&encode_player_chat(
            0,
            &uuid,
            "AlphaProbe",
            "hello from alpha",
            1788380644655,
        ));
        assert_eq!(id, CB_PLAYER_CHAT);

        let head = hex(concat!(
            "00",                                 // globalIndex
            "ac964522d95e3ef0b4524e380e6762bf",   // senderUuid
            "00",                                 // index
            "00",                                 // signature: none
            "1068656c6c6f2066726f6d20616c706861", // plainMessage
            "000001a063cb052f",                   // timestamp
            "0000000000000000",                   // salt
            "00",                                 // previousMessages: none
            "00",                                 // unsignedChatContent: none
            "00",                                 // filterType: PASS_THROUGH
            "01",                                 // chat_type holder = id 0 + 1
        ));
        assert_eq!(&body[..head.len()], &head[..], "fixed fields match vanilla");
        // networkName as a bare NBT string, then networkTargetName = none.
        let mut tail = encode_text("AlphaProbe");
        tail.push(0x00);
        assert_eq!(&body[head.len()..], &tail[..]);
    }

    #[test]
    fn player_chat_global_index_and_uuid_are_written_verbatim() {
        let uuid = [0x11u8; 16];
        let (_, body) = payload(&encode_player_chat(5, &uuid, "N", "m", 0));
        assert_eq!(body[0], 0x05, "globalIndex varint");
        assert_eq!(&body[1..17], &uuid, "senderUuid");
    }

    #[test]
    fn profileless_chat_uses_the_registry_holder_offset() {
        let (id, body) = payload(&encode_profileless_chat(
            "waves",
            CHAT_TYPE_EMOTE_COMMAND,
            "AlphaProbe",
        ));
        assert_eq!(id, CB_PROFILELESS_CHAT);
        let mut want = encode_text("waves");
        // emote_command is registry id 1, so the holder varint is 2 — exactly
        // what the capture of vanilla's `/me waves` shows.
        want.push(0x02);
        want.extend_from_slice(&encode_text("AlphaProbe"));
        want.push(0x00);
        assert_eq!(body, want);

        // /say is registry id 4 -> holder 5, also matching the capture.
        let (_, say) = payload(&encode_profileless_chat("hi", CHAT_TYPE_SAY_COMMAND, "A"));
        let holder_at = encode_text("hi").len();
        assert_eq!(say[holder_at], 0x05);
    }

    #[test]
    fn chat_message_payload_is_parsed_from_its_leading_string() {
        // message + timestamp + salt + no signature + offset + acknowledged + checksum
        let mut data = Vec::new();
        write_string("hello world", &mut data);
        data.extend_from_slice(&1234i64.to_be_bytes());
        data.extend_from_slice(&0i64.to_be_bytes());
        data.push(0x00);
        encode_varint(0, &mut data);
        data.extend_from_slice(&[0, 0, 0]);
        data.push(0x00);
        assert_eq!(
            parse_chat_input(SB_CHAT_MESSAGE, &data),
            Some(ChatInput::Message("hello world".to_string()))
        );
    }

    #[test]
    fn chat_command_payload_is_a_bare_string() {
        let mut data = Vec::new();
        write_string("gamemode creative", &mut data);
        assert_eq!(
            parse_chat_input(SB_CHAT_COMMAND, &data),
            Some(ChatInput::Command("gamemode creative".to_string()))
        );
        // The signed variant starts with the same string field.
        assert_eq!(
            parse_chat_input(SB_CHAT_COMMAND_SIGNED, &data),
            Some(ChatInput::Command("gamemode creative".to_string()))
        );
    }

    #[test]
    fn unrelated_and_malformed_packets_are_ignored() {
        assert_eq!(parse_chat_input(0x1e, &[1, 2, 3]), None);
        assert_eq!(parse_chat_input(SB_CHAT_MESSAGE, &[]), None);
        // A length prefix longer than the payload must not panic.
        assert_eq!(parse_chat_input(SB_CHAT_COMMAND, &[0x40, b'a']), None);
    }

    #[test]
    fn control_characters_and_colour_codes_are_stripped() {
        let mut data = Vec::new();
        write_string("bad\nline\u{00a7}cred", &mut data);
        data.extend_from_slice(&0i64.to_be_bytes());
        data.extend_from_slice(&0i64.to_be_bytes());
        data.push(0x00);
        encode_varint(0, &mut data);
        data.extend_from_slice(&[0, 0, 0]);
        data.push(0x00);
        assert_eq!(
            parse_chat_input(SB_CHAT_MESSAGE, &data),
            Some(ChatInput::Message("badlinecred".to_string()))
        );
    }

    #[test]
    fn message_acknowledgement_is_a_single_varint() {
        assert_eq!(read_message_acknowledgement(&[0x07]).expect("count"), 7);
    }
}
