//! Player state packets: health, hunger, world time, abilities and gamemode.
//!
//! # Packet ids (protocol 776)
//!
//! Taken from `target/protocol_26_1.json` and confirmed against live captures
//! (`data/play_packets.bin` for join, `scripts/capture_chat_op.py` and
//! `scripts/capture_time.py` for changes made at runtime):
//!
//! | direction | name                | id     | captured bytes |
//! |-----------|---------------------|--------|----------------|
//! | S→C | `abilities`         | `0x40` | `00 3d4ccccd 3dcccccd` (survival) |
//! | S→C | `game_state_change` | `0x26` | `03 3f800000` (gamemode -> creative) |
//! | S→C | `update_health`     | `0x68` | `41a00000 14 40a00000` (20 / 20 / 5) |
//! | S→C | `update_time`       | `0x71` | see [`encode_update_time`] |
//!
//! Note the ids the task sketch guessed (`set_health` 0x67, `update_time` 0x66,
//! `player_abilities` 0x36) belong to other packets in 776: `0x67` is
//! `experience`, `0x66` is `entity_equipment` and `0x36` is `entity_move_look`.
//! The ids above are the real ones.

use bcore_core::varint::{encode_varint, encode_varlong};

use crate::packet::write_packet;

/// Clientbound `game_state_change`.
pub const CB_GAME_STATE_CHANGE: i32 = 0x26;
/// Clientbound `abilities` (the "player abilities" packet).
pub const CB_ABILITIES: i32 = 0x40;
/// Clientbound `update_health` (health + food + saturation).
pub const CB_UPDATE_HEALTH: i32 = 0x68;
/// Clientbound `update_time`.
pub const CB_UPDATE_TIME: i32 = 0x71;

/// `game_state_change` reason: the player's gamemode changed.
pub const GAME_STATE_CHANGE_GAMEMODE: u8 = 3;

/// Flying speed vanilla sends in `abilities` (0.05).
pub const DEFAULT_FLYING_SPEED: f32 = 0.05;
/// Walking speed vanilla sends in `abilities` (0.1).
pub const DEFAULT_WALKING_SPEED: f32 = 0.1;

/// `update_time` clock id carrying the day time (0..24000 within a day).
///
/// Vanilla sends two clock entries at join and only clock 0 when `/time set`
/// runs; `scripts/capture_time.py` shows clock 0 jumping to 61000 / 73000 /
/// 6000 / 18000 for `/time set night|day|6000|midnight`, i.e. clock 0 is the
/// day-time clock (mod 24000 = 13000 / 1000 / 6000 / 18000).
pub const CLOCK_DAY_TIME: i32 = 0;
/// `update_time` clock id carrying the total world age.
pub const CLOCK_WORLD_AGE: i32 = 1;

/// Day time (ticks within a Minecraft day) for `/time set day`.
pub const TIME_DAY: i64 = 1000;
/// Day time for `/time set noon`.
pub const TIME_NOON: i64 = 6000;
/// Day time for `/time set night`.
pub const TIME_NIGHT: i64 = 13000;
/// Day time for `/time set midnight`.
pub const TIME_MIDNIGHT: i64 = 18000;
/// Ticks in one Minecraft day.
pub const TICKS_PER_DAY: i64 = 24000;

/// A player's gamemode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl GameMode {
    /// The protocol id (as used by `game_state_change` and `player_info`).
    pub fn id(self) -> i32 {
        match self {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 3,
        }
    }

    /// The lowercase name the client and `/gamemode` use.
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "survival",
            GameMode::Creative => "creative",
            GameMode::Adventure => "adventure",
            GameMode::Spectator => "spectator",
        }
    }

    /// Parse a `/gamemode` argument (name or numeric id).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "survival" | "s" | "0" => Some(GameMode::Survival),
            "creative" | "c" | "1" => Some(GameMode::Creative),
            "adventure" | "a" | "2" => Some(GameMode::Adventure),
            "spectator" | "sp" | "3" => Some(GameMode::Spectator),
            _ => None,
        }
    }

    /// Build a [`GameMode`] from its protocol id.
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(GameMode::Survival),
            1 => Some(GameMode::Creative),
            2 => Some(GameMode::Adventure),
            3 => Some(GameMode::Spectator),
            _ => None,
        }
    }

    /// The `abilities` flags byte for this gamemode.
    ///
    /// Bits: `0x01` invulnerable, `0x02` flying, `0x04` allow flying,
    /// `0x08` creative mode (instant break). Captured values: survival `0x00`,
    /// creative `0x0d`, spectator `0x07`.
    pub fn ability_flags(self) -> u8 {
        match self {
            GameMode::Survival | GameMode::Adventure => 0x00,
            // invulnerable + allow flying + creative (flying itself is not
            // forced on: the client starts grounded and toggles it).
            GameMode::Creative => 0x0d,
            // invulnerable + flying + allow flying
            GameMode::Spectator => 0x07,
        }
    }
}

/// Encode `update_health` (0x68): `f32 health`, `varint food`, `f32 saturation`.
pub fn encode_update_health(health: f32, food: i32, saturation: f32) -> Vec<u8> {
    let mut data = Vec::with_capacity(9);
    data.extend_from_slice(&health.to_be_bytes());
    encode_varint(food, &mut data);
    data.extend_from_slice(&saturation.to_be_bytes());
    let mut out = Vec::new();
    write_packet(&mut out, CB_UPDATE_HEALTH, &data);
    out
}

/// Encode a full-health `update_health`: 20 health, 20 food, 5.0 saturation —
/// byte-identical to what vanilla sends at join (`41a00000 14 40a00000`).
pub fn encode_full_health() -> Vec<u8> {
    encode_update_health(20.0, 20, 5.0)
}

/// One entry of the `update_time` `clockUpdates` array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockUpdate {
    /// Clock id: [`CLOCK_DAY_TIME`] or [`CLOCK_WORLD_AGE`].
    pub id: i32,
    /// Absolute tick count for that clock.
    pub total_ticks: i64,
    /// Sub-tick fraction; vanilla sends 0.0.
    pub partial_tick: f32,
    /// Tick rate multiplier; 1.0 means normal speed, 0.0 freezes the clock.
    pub rate: f32,
}

impl ClockUpdate {
    /// A normal-speed clock at `total_ticks`.
    pub fn running(id: i32, total_ticks: i64) -> Self {
        Self {
            id,
            total_ticks,
            partial_tick: 0.0,
            rate: 1.0,
        }
    }
}

/// Encode `update_time` (0x71).
///
/// ```text
/// age: i64
/// clockUpdates: varint count, then per entry
///     { id varint, totalTicks varlong, partialTick f32, rate f32 }
/// ```
///
/// Captured example (`/time set 6000`):
/// `00000000000043cd 01 00 f02e 00000000 3f800000` — age, one update, clock 0,
/// varlong 6000, partialTick 0.0, rate 1.0.
pub fn encode_update_time(age: i64, updates: &[ClockUpdate]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&age.to_be_bytes());
    encode_varint(updates.len() as i32, &mut data);
    for update in updates {
        encode_varint(update.id, &mut data);
        encode_varlong(update.total_ticks, &mut data);
        data.extend_from_slice(&update.partial_tick.to_be_bytes());
        data.extend_from_slice(&update.rate.to_be_bytes());
    }
    let mut out = Vec::new();
    write_packet(&mut out, CB_UPDATE_TIME, &data);
    out
}

/// Encode the join-time `update_time`: both clocks running at `age`, like vanilla.
pub fn encode_time_of_day(age: i64, day_time: i64) -> Vec<u8> {
    encode_update_time(
        age,
        &[
            ClockUpdate::running(CLOCK_WORLD_AGE, age),
            ClockUpdate::running(CLOCK_DAY_TIME, day_time),
        ],
    )
}

/// Encode a `/time set` result: only the day-time clock moves, as vanilla does.
pub fn encode_set_day_time(age: i64, day_time: i64) -> Vec<u8> {
    encode_update_time(age, &[ClockUpdate::running(CLOCK_DAY_TIME, day_time)])
}

/// Encode `abilities` (0x40): `i8 flags`, `f32 flyingSpeed`, `f32 walkingSpeed`.
pub fn encode_abilities(flags: u8, flying_speed: f32, walking_speed: f32) -> Vec<u8> {
    let mut data = Vec::with_capacity(9);
    data.push(flags);
    data.extend_from_slice(&flying_speed.to_be_bytes());
    data.extend_from_slice(&walking_speed.to_be_bytes());
    let mut out = Vec::new();
    write_packet(&mut out, CB_ABILITIES, &data);
    out
}

/// Encode `abilities` for a gamemode using vanilla's default speeds.
pub fn encode_abilities_for(mode: GameMode) -> Vec<u8> {
    encode_abilities(
        mode.ability_flags(),
        DEFAULT_FLYING_SPEED,
        DEFAULT_WALKING_SPEED,
    )
}

/// Encode `game_state_change` (0x26): `u8 reason`, `f32 value`.
pub fn encode_game_state_change(reason: u8, value: f32) -> Vec<u8> {
    let mut data = Vec::with_capacity(5);
    data.push(reason);
    data.extend_from_slice(&value.to_be_bytes());
    let mut out = Vec::new();
    write_packet(&mut out, CB_GAME_STATE_CHANGE, &data);
    out
}

/// Encode the gamemode-change `game_state_change` (reason 3, value = mode id).
pub fn encode_gamemode_change(mode: GameMode) -> Vec<u8> {
    encode_game_state_change(GAME_STATE_CHANGE_GAMEMODE, mode.id() as f32)
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

    fn payload(frame: &[u8]) -> (i32, Vec<u8>) {
        let (len, n) = decode_varint(frame).expect("length");
        assert_eq!(frame.len(), n + len as usize, "frame length must be exact");
        let (id, m) = decode_varint(&frame[n..]).expect("id");
        (id, frame[n + m..].to_vec())
    }

    #[test]
    fn full_health_matches_the_captured_vanilla_bytes() {
        let (id, body) = payload(&encode_full_health());
        assert_eq!(id, CB_UPDATE_HEALTH);
        // From data/play_packets.bin: 41a00000 (20.0f) 14 (varint 20) 40a00000 (5.0f).
        assert_eq!(body, hex("41a000001440a00000"));
    }

    #[test]
    fn update_health_encodes_each_field_in_order() {
        let (_, body) = payload(&encode_update_health(17.0, 20, 5.0));
        // Captured after vanilla dealt 3 damage: 418800001440a00000.
        assert_eq!(body, hex("418800001440a00000"));
        // Food is a varint, so 200 takes two bytes and shifts saturation.
        let (_, body) = payload(&encode_update_health(1.0, 200, 0.0));
        assert_eq!(body, hex("3f800000c80100000000"));
    }

    #[test]
    fn update_time_matches_the_captured_time_set_packet() {
        // Captured reply to `/time set 6000`: age 17357, one clock update.
        let (id, body) = payload(&encode_set_day_time(17357, 6000));
        assert_eq!(id, CB_UPDATE_TIME);
        assert_eq!(body, hex("00000000000043cd0100f02e000000003f800000"));
    }

    #[test]
    fn join_time_sends_both_clocks_like_vanilla() {
        let (_, body) = payload(&encode_time_of_day(4878, 4878));
        // Captured at join in data/play_packets.bin, clock order 1 then 0.
        assert_eq!(
            body,
            hex("000000000000130e02018e26000000003f800000008e26000000003f800000")
        );
    }

    #[test]
    fn clock_ids_and_day_times_follow_the_capture() {
        // /time set night -> clock 0 = 13000 (mod a day); /time set day -> 1000.
        let (_, night) = payload(&encode_set_day_time(0, TIME_NIGHT));
        assert_eq!(night[8], 0x01, "one clock update");
        assert_eq!(night[9], CLOCK_DAY_TIME as u8, "day-time clock");
        let mut want = Vec::new();
        encode_varlong(TIME_NIGHT, &mut want);
        assert_eq!(&night[10..10 + want.len()], &want[..]);
        assert_eq!(TIME_NIGHT % TICKS_PER_DAY, 13000);
        assert_eq!(TIME_DAY % TICKS_PER_DAY, 1000);
        assert_eq!(TIME_NOON, 6000);
        assert_eq!(TIME_MIDNIGHT, 18000);
    }

    #[test]
    fn large_tick_counts_use_varlong_encoding() {
        // Vanilla sent totalTicks 73000 for `/time set day` after 3 days.
        let (_, body) = payload(&encode_set_day_time(17307, 73000));
        assert_eq!(body, hex("000000000000439b0100a8ba04000000003f800000"));
    }

    #[test]
    fn abilities_match_the_captured_bytes_per_gamemode() {
        // Survival at join: 00 3d4ccccd 3dcccccd.
        let (id, body) = payload(&encode_abilities_for(GameMode::Survival));
        assert_eq!(id, CB_ABILITIES);
        assert_eq!(body, hex("003d4ccccd3dcccccd"));
        // /gamemode creative: flags 0x0d.
        let (_, body) = payload(&encode_abilities_for(GameMode::Creative));
        assert_eq!(body, hex("0d3d4ccccd3dcccccd"));
        // /gamemode spectator: flags 0x07.
        let (_, body) = payload(&encode_abilities_for(GameMode::Spectator));
        assert_eq!(body, hex("073d4ccccd3dcccccd"));
        // Adventure behaves like survival on the ability flags.
        let (_, body) = payload(&encode_abilities_for(GameMode::Adventure));
        assert_eq!(body, hex("003d4ccccd3dcccccd"));
    }

    #[test]
    fn ability_flag_bits_have_the_documented_meaning() {
        assert_eq!(
            GameMode::Creative.ability_flags() & 0x01,
            0x01,
            "invulnerable"
        );
        assert_eq!(
            GameMode::Creative.ability_flags() & 0x04,
            0x04,
            "allow flying"
        );
        assert_eq!(GameMode::Creative.ability_flags() & 0x08, 0x08, "creative");
        assert_eq!(GameMode::Spectator.ability_flags() & 0x02, 0x02, "flying");
        assert_eq!(GameMode::Survival.ability_flags(), 0x00);
    }

    #[test]
    fn gamemode_change_matches_the_captured_game_state_change() {
        // /gamemode creative -> 03 3f800000 ; survival -> 03 00000000 ;
        // spectator -> 03 40400000.
        let (id, body) = payload(&encode_gamemode_change(GameMode::Creative));
        assert_eq!(id, CB_GAME_STATE_CHANGE);
        assert_eq!(body, hex("033f800000"));
        let (_, body) = payload(&encode_gamemode_change(GameMode::Survival));
        assert_eq!(body, hex("0300000000"));
        let (_, body) = payload(&encode_gamemode_change(GameMode::Spectator));
        assert_eq!(body, hex("0340400000"));
        let (_, body) = payload(&encode_gamemode_change(GameMode::Adventure));
        assert_eq!(body, hex("0340000000"), "adventure = 2.0f");
    }

    #[test]
    fn gamemode_names_and_ids_round_trip() {
        for mode in [
            GameMode::Survival,
            GameMode::Creative,
            GameMode::Adventure,
            GameMode::Spectator,
        ] {
            assert_eq!(GameMode::parse(mode.name()), Some(mode));
            assert_eq!(GameMode::parse(&mode.id().to_string()), Some(mode));
        }
        assert_eq!(GameMode::parse("CREATIVE"), Some(GameMode::Creative));
        assert_eq!(GameMode::parse(" c "), Some(GameMode::Creative));
        assert_eq!(GameMode::parse("hardcore"), None);
    }
}
