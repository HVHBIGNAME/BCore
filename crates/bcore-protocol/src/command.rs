//! Command execution: parse a `chat_command` payload and decide what to send.
//!
//! Dispatch is deliberately pure. [`execute`] takes the command text plus a
//! snapshot of who ran it and returns a [`CommandOutcome`] describing the
//! packets to send and the state changes to apply; it performs no I/O and needs
//! no socket, so every command is unit-testable without a client.
//!
//! Commands implemented: `/help`, `/list`, `/me`, `/say`, `/spawn`, `/gamemode`,
//! `/tp`, `/seed`, `/time set`, `/kick`, `/stop`. The advertised tree in
//! [`crate::commands::bcore_command_tree`] matches this set exactly.

use crate::chat::{
    encode_profileless_chat, encode_system_chat, encode_system_message, CHAT_TYPE_EMOTE_COMMAND,
    CHAT_TYPE_SAY_COMMAND,
};
use crate::gameplay::{GameMode, TIME_DAY, TIME_MIDNIGHT, TIME_NIGHT, TIME_NOON};
use crate::nbt::Component;

/// Vanilla's colour for command errors.
const ERROR_COLOR: &str = "red";
/// Colour used for informational server output.
const INFO_COLOR: &str = "yellow";

/// Where a packet produced by a command should go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Only the player who ran the command.
    Sender,
    /// Every online player, sender included.
    Everyone,
    /// Every online player except the sender.
    Others,
}

/// One packet a command wants sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundPacket {
    pub destination: Destination,
    /// Fully framed packet bytes (length prefix + id + payload).
    pub bytes: Vec<u8>,
}

/// A state change the play loop must apply after running a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Switch the sender's gamemode (abilities + `game_state_change`).
    SetGameMode(GameMode),
    /// Teleport the sender to these coordinates.
    Teleport { x: f64, y: f64, z: f64 },
    /// Set the world's day time and tell everyone.
    SetDayTime(i64),
    /// Disconnect the named player.
    Kick(String),
    /// Stop the whole server.
    Stop,
}

/// Everything a command produced: packets to send and effects to apply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandOutcome {
    pub packets: Vec<OutboundPacket>,
    pub effects: Vec<Effect>,
}

impl CommandOutcome {
    fn to_sender(bytes: Vec<u8>) -> Self {
        Self {
            packets: vec![OutboundPacket {
                destination: Destination::Sender,
                bytes,
            }],
            effects: Vec::new(),
        }
    }

    fn to_everyone(bytes: Vec<u8>) -> Self {
        Self {
            packets: vec![OutboundPacket {
                destination: Destination::Everyone,
                bytes,
            }],
            effects: Vec::new(),
        }
    }

    fn with_effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    /// An error reply in vanilla's red.
    fn error(message: &str) -> Self {
        Self::to_sender(encode_system_chat(
            &Component::colored(message, ERROR_COLOR),
            false,
        ))
    }

    /// An informational reply in yellow.
    fn info(message: &str) -> Self {
        Self::to_sender(encode_system_chat(
            &Component::colored(message, INFO_COLOR),
            false,
        ))
    }
}

/// Who ran the command and what the server knows about the world.
#[derive(Debug, Clone)]
pub struct CommandContext<'a> {
    pub sender_name: &'a str,
    /// Online player names, sorted.
    pub online: &'a [String],
    /// Maximum player slots, as announced in the status response.
    pub max_players: usize,
    /// The world seed reported by `/seed`.
    pub seed: i64,
    /// The spawn position `/spawn` teleports to.
    pub spawn: (f64, f64, f64),
}

/// The `/help` text, one line per command. Ordered, so output is deterministic.
pub const HELP_LINES: &[&str] = &[
    "/help [<command>] - list commands or describe one",
    "/list - who is online",
    "/me <action> - emote to everyone",
    "/say <message> - broadcast as the server",
    "/spawn - teleport to the world spawn",
    "/gamemode <survival|creative|adventure|spectator> - change your gamemode",
    "/tp <x> <y> <z> - teleport to coordinates",
    "/seed - show the world seed",
    "/time set <day|noon|night|midnight> - set the world time",
    "/kick <player> - disconnect a player",
    "/stop - stop the server",
];

/// Execute a command (without its leading slash).
///
/// Unknown commands and bad arguments produce a red `system_chat` reply, the
/// same way vanilla reports `command.unknown.command`.
pub fn execute(command: &str, ctx: &CommandContext<'_>) -> CommandOutcome {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return CommandOutcome::error("Unknown or incomplete command");
    }
    let (name, rest) = match trimmed.split_once(' ') {
        Some((name, rest)) => (name, rest.trim()),
        None => (trimmed, ""),
    };

    match name.to_ascii_lowercase().as_str() {
        "help" | "?" => help(rest),
        "list" => list(ctx),
        "me" => me(rest, ctx),
        "say" => say(rest, ctx),
        "spawn" => spawn(ctx),
        "gamemode" | "gm" => gamemode(rest),
        "tp" | "teleport" => teleport(rest),
        "seed" => seed(ctx),
        "time" => time(rest),
        "kick" => kick(rest, ctx),
        "stop" => stop(),
        other => CommandOutcome::error(&format!("Unknown or incomplete command: /{other}")),
    }
}

fn help(argument: &str) -> CommandOutcome {
    if argument.is_empty() {
        let mut packets = Vec::with_capacity(HELP_LINES.len() + 1);
        packets.push(OutboundPacket {
            destination: Destination::Sender,
            bytes: encode_system_chat(
                &Component::colored(
                    format!("BCore commands ({}):", HELP_LINES.len()),
                    INFO_COLOR,
                ),
                false,
            ),
        });
        for line in HELP_LINES {
            packets.push(OutboundPacket {
                destination: Destination::Sender,
                bytes: encode_system_message(line),
            });
        }
        return CommandOutcome {
            packets,
            effects: Vec::new(),
        };
    }

    let wanted = argument.trim_start_matches('/').to_ascii_lowercase();
    match HELP_LINES
        .iter()
        .find(|line| line[1..].starts_with(&wanted))
    {
        Some(line) => CommandOutcome::to_sender(encode_system_message(line)),
        None => CommandOutcome::error(&format!("Unknown command: /{wanted}")),
    }
}

fn list(ctx: &CommandContext<'_>) -> CommandOutcome {
    let names = ctx.online.join(", ");
    CommandOutcome::info(&format!(
        "There are {} of a max of {} players online: {names}",
        ctx.online.len(),
        ctx.max_players
    ))
}

fn me(action: &str, ctx: &CommandContext<'_>) -> CommandOutcome {
    if action.is_empty() {
        return CommandOutcome::error("Usage: /me <action>");
    }
    // Vanilla emits profileless_chat with chat_type emote_command for /me.
    CommandOutcome::to_everyone(encode_profileless_chat(
        action,
        CHAT_TYPE_EMOTE_COMMAND,
        ctx.sender_name,
    ))
}

fn say(message: &str, ctx: &CommandContext<'_>) -> CommandOutcome {
    if message.is_empty() {
        return CommandOutcome::error("Usage: /say <message>");
    }
    // Vanilla uses chat_type say_command for /say.
    CommandOutcome::to_everyone(encode_profileless_chat(
        message,
        CHAT_TYPE_SAY_COMMAND,
        ctx.sender_name,
    ))
}

fn spawn(ctx: &CommandContext<'_>) -> CommandOutcome {
    let (x, y, z) = ctx.spawn;
    CommandOutcome::info(&format!("Teleported to spawn ({x}, {y}, {z})"))
        .with_effect(Effect::Teleport { x, y, z })
}

fn gamemode(argument: &str) -> CommandOutcome {
    if argument.is_empty() {
        return CommandOutcome::error("Usage: /gamemode <survival|creative|adventure|spectator>");
    }
    match GameMode::parse(argument) {
        Some(mode) => CommandOutcome::info(&format!("Set own game mode to {}", mode.name()))
            .with_effect(Effect::SetGameMode(mode)),
        None => CommandOutcome::error(&format!("Unknown game mode: {argument}")),
    }
}

fn teleport(argument: &str) -> CommandOutcome {
    let parts: Vec<&str> = argument.split_whitespace().collect();
    if parts.len() != 3 {
        return CommandOutcome::error("Usage: /tp <x> <y> <z>");
    }
    let mut coords = [0.0f64; 3];
    for (slot, text) in coords.iter_mut().zip(&parts) {
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => *slot = value,
            _ => return CommandOutcome::error(&format!("Invalid coordinate: {text}")),
        }
    }
    let [x, y, z] = coords;
    CommandOutcome::info(&format!("Teleported to {x}, {y}, {z}")).with_effect(Effect::Teleport {
        x,
        y,
        z,
    })
}

fn seed(ctx: &CommandContext<'_>) -> CommandOutcome {
    CommandOutcome::info(&format!("Seed: [{}]", ctx.seed))
}

fn time(argument: &str) -> CommandOutcome {
    let mut parts = argument.split_whitespace();
    match parts.next() {
        Some("set") => {}
        Some(other) => {
            return CommandOutcome::error(&format!("Unknown time subcommand: {other}"));
        }
        None => return CommandOutcome::error("Usage: /time set <day|noon|night|midnight>"),
    }
    let when = match parts.next() {
        Some(when) => when,
        None => return CommandOutcome::error("Usage: /time set <day|noon|night|midnight>"),
    };
    let ticks = match when.to_ascii_lowercase().as_str() {
        "day" => TIME_DAY,
        "noon" => TIME_NOON,
        "night" => TIME_NIGHT,
        "midnight" => TIME_MIDNIGHT,
        // `/time set <ticks>` also works, matching vanilla.
        numeric => match numeric.parse::<i64>() {
            Ok(ticks) if ticks >= 0 => ticks,
            _ => return CommandOutcome::error(&format!("Invalid time: {when}")),
        },
    };
    CommandOutcome::info(&format!("Set the time to {ticks}")).with_effect(Effect::SetDayTime(ticks))
}

fn kick(argument: &str, ctx: &CommandContext<'_>) -> CommandOutcome {
    let target = argument.split_whitespace().next().unwrap_or("");
    if target.is_empty() {
        return CommandOutcome::error("Usage: /kick <player>");
    }
    let known = ctx
        .online
        .iter()
        .find(|name| name.eq_ignore_ascii_case(target));
    match known {
        Some(name) => {
            CommandOutcome::info(&format!("Kicked {name}")).with_effect(Effect::Kick(name.clone()))
        }
        None => CommandOutcome::error(&format!("No player was found: {target}")),
    }
}

fn stop() -> CommandOutcome {
    CommandOutcome::info("Stopping the server").with_effect(Effect::Stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{CB_PROFILELESS_CHAT, CB_SYSTEM_CHAT};
    use bcore_core::varint::decode_varint;

    const ONLINE: [&str; 2] = ["AlphaProbe", "BetaProbe"];

    fn ctx<'a>(online: &'a [String]) -> CommandContext<'a> {
        CommandContext {
            sender_name: "AlphaProbe",
            online,
            max_players: 20,
            seed: 1027236290406173232,
            spawn: (10.5, -60.0, -3.5),
        }
    }

    fn online() -> Vec<String> {
        ONLINE.iter().map(|s| s.to_string()).collect()
    }

    /// Decode a framed packet into `(id, payload)`.
    fn split(frame: &[u8]) -> (i32, Vec<u8>) {
        let (len, n) = decode_varint(frame).expect("length");
        assert_eq!(frame.len(), n + len as usize);
        let (id, m) = decode_varint(&frame[n..]).expect("id");
        (id, frame[n + m..].to_vec())
    }

    /// The literal text carried by a `system_chat` packet, whatever its NBT shape.
    fn chat_text(frame: &[u8]) -> String {
        let (id, body) = split(frame);
        assert_eq!(id, CB_SYSTEM_CHAT, "expected system_chat");
        // Both NBT shapes end their last string with the message; scan for the
        // longest printable run so the assertion does not depend on styling.
        let mut best = String::new();
        let mut at = 0usize;
        while at + 2 <= body.len() {
            let len = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
            if len > 0 && at + 2 + len <= body.len() {
                if let Ok(s) = std::str::from_utf8(&body[at + 2..at + 2 + len]) {
                    if s.chars().all(|c| !c.is_control()) && s.len() > best.len() {
                        best = s.to_string();
                    }
                }
            }
            at += 1;
        }
        best
    }

    #[test]
    fn help_lists_every_implemented_command_to_the_sender_only() {
        let online = online();
        let outcome = execute("help", &ctx(&online));
        assert!(outcome.effects.is_empty());
        // A header plus one line per command.
        assert_eq!(outcome.packets.len(), HELP_LINES.len() + 1);
        assert!(outcome
            .packets
            .iter()
            .all(|p| p.destination == Destination::Sender));
        let texts: Vec<String> = outcome
            .packets
            .iter()
            .map(|p| chat_text(&p.bytes))
            .collect();
        assert!(texts[0].contains("BCore commands"));
        for command in [
            "/help",
            "/list",
            "/me",
            "/say",
            "/spawn",
            "/gamemode",
            "/tp",
            "/seed",
            "/time set",
            "/kick",
            "/stop",
        ] {
            assert!(
                texts.iter().any(|t| t.starts_with(command)),
                "missing help for {command}"
            );
        }
    }

    #[test]
    fn help_with_an_argument_describes_one_command() {
        let online = online();
        let outcome = execute("help list", &ctx(&online));
        assert_eq!(outcome.packets.len(), 1);
        assert!(chat_text(&outcome.packets[0].bytes).starts_with("/list"));
        // A leading slash is tolerated.
        let outcome = execute("help /seed", &ctx(&online));
        assert!(chat_text(&outcome.packets[0].bytes).starts_with("/seed"));
        // Unknown topics are an error.
        let outcome = execute("help fly", &ctx(&online));
        assert!(chat_text(&outcome.packets[0].bytes).contains("Unknown command"));
    }

    #[test]
    fn list_reports_the_sorted_online_players_and_the_slot_count() {
        let online = online();
        let outcome = execute("list", &ctx(&online));
        let text = chat_text(&outcome.packets[0].bytes);
        assert_eq!(
            text,
            "There are 2 of a max of 20 players online: AlphaProbe, BetaProbe"
        );
        assert_eq!(outcome.packets[0].destination, Destination::Sender);
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn me_broadcasts_profileless_chat_with_the_emote_chat_type() {
        let online = online();
        let outcome = execute("me waves at everyone", &ctx(&online));
        assert_eq!(outcome.packets.len(), 1);
        assert_eq!(outcome.packets[0].destination, Destination::Everyone);
        let (id, body) = split(&outcome.packets[0].bytes);
        assert_eq!(id, CB_PROFILELESS_CHAT);
        // NBT string root, then the chat-type holder = emote_command(1) + 1.
        assert_eq!(body[0], crate::nbt::TAG_STRING);
        let msg_len = u16::from_be_bytes([body[1], body[2]]) as usize;
        assert_eq!(&body[3..3 + msg_len], b"waves at everyone");
        assert_eq!(body[3 + msg_len], (CHAT_TYPE_EMOTE_COMMAND + 1) as u8);
    }

    #[test]
    fn say_broadcasts_with_the_say_chat_type() {
        let online = online();
        let outcome = execute("say server going down", &ctx(&online));
        assert_eq!(outcome.packets[0].destination, Destination::Everyone);
        let (id, body) = split(&outcome.packets[0].bytes);
        assert_eq!(id, CB_PROFILELESS_CHAT);
        let msg_len = u16::from_be_bytes([body[1], body[2]]) as usize;
        assert_eq!(&body[3..3 + msg_len], b"server going down");
        assert_eq!(body[3 + msg_len], (CHAT_TYPE_SAY_COMMAND + 1) as u8);
    }

    #[test]
    fn me_and_say_require_an_argument() {
        let online = online();
        for command in ["me", "say", "me   "] {
            let outcome = execute(command, &ctx(&online));
            assert!(
                chat_text(&outcome.packets[0].bytes).starts_with("Usage:"),
                "{command} should explain itself"
            );
            assert!(outcome.effects.is_empty());
        }
    }

    #[test]
    fn spawn_teleports_to_the_context_spawn() {
        let online = online();
        let outcome = execute("spawn", &ctx(&online));
        assert_eq!(
            outcome.effects,
            vec![Effect::Teleport {
                x: 10.5,
                y: -60.0,
                z: -3.5
            }]
        );
        assert!(chat_text(&outcome.packets[0].bytes).contains("spawn"));
    }

    #[test]
    fn gamemode_accepts_names_abbreviations_and_ids() {
        let online = online();
        for (argument, want) in [
            ("survival", GameMode::Survival),
            ("creative", GameMode::Creative),
            ("spectator", GameMode::Spectator),
            ("adventure", GameMode::Adventure),
            ("c", GameMode::Creative),
            ("1", GameMode::Creative),
            ("CREATIVE", GameMode::Creative),
        ] {
            let outcome = execute(&format!("gamemode {argument}"), &ctx(&online));
            assert_eq!(
                outcome.effects,
                vec![Effect::SetGameMode(want)],
                "/gamemode {argument}"
            );
        }
        // `gm` is accepted as an alias.
        let outcome = execute("gm creative", &ctx(&online));
        assert_eq!(
            outcome.effects,
            vec![Effect::SetGameMode(GameMode::Creative)]
        );
    }

    #[test]
    fn gamemode_rejects_unknown_modes_without_side_effects() {
        let online = online();
        for command in ["gamemode", "gamemode hardcore", "gamemode 9"] {
            let outcome = execute(command, &ctx(&online));
            assert!(outcome.effects.is_empty(), "{command} must not take effect");
            let text = chat_text(&outcome.packets[0].bytes);
            assert!(text.contains("Usage:") || text.contains("Unknown game mode"));
        }
    }

    #[test]
    fn tp_parses_three_coordinates_including_negatives_and_decimals() {
        let online = online();
        let outcome = execute("tp 100.5 -60 -3.25", &ctx(&online));
        assert_eq!(
            outcome.effects,
            vec![Effect::Teleport {
                x: 100.5,
                y: -60.0,
                z: -3.25
            }]
        );
        // `teleport` is accepted as an alias.
        let outcome = execute("teleport 1 2 3", &ctx(&online));
        assert_eq!(
            outcome.effects,
            vec![Effect::Teleport {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }]
        );
    }

    #[test]
    fn tp_rejects_wrong_arity_and_non_finite_coordinates() {
        let online = online();
        for command in [
            "tp",
            "tp 1",
            "tp 1 2",
            "tp 1 2 3 4",
            "tp a b c",
            "tp 1 NaN 3",
            "tp 1 inf 3",
        ] {
            let outcome = execute(command, &ctx(&online));
            assert!(outcome.effects.is_empty(), "{command} must not teleport");
        }
    }

    #[test]
    fn seed_reports_the_world_seed() {
        let online = online();
        let outcome = execute("seed", &ctx(&online));
        assert!(chat_text(&outcome.packets[0].bytes).contains("1027236290406173232"));
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn time_set_maps_the_vanilla_time_markers() {
        let online = online();
        for (argument, ticks) in [
            ("day", TIME_DAY),
            ("noon", TIME_NOON),
            ("night", TIME_NIGHT),
            ("midnight", TIME_MIDNIGHT),
            ("6000", 6000),
        ] {
            let outcome = execute(&format!("time set {argument}"), &ctx(&online));
            assert_eq!(
                outcome.effects,
                vec![Effect::SetDayTime(ticks)],
                "/time set {argument}"
            );
        }
    }

    #[test]
    fn time_rejects_bad_input() {
        let online = online();
        for command in [
            "time",
            "time set",
            "time query",
            "time set later",
            "time set -5",
        ] {
            let outcome = execute(command, &ctx(&online));
            assert!(outcome.effects.is_empty(), "{command} must not change time");
        }
    }

    #[test]
    fn kick_only_targets_players_who_are_online() {
        let online = online();
        let outcome = execute("kick BetaProbe", &ctx(&online));
        assert_eq!(outcome.effects, vec![Effect::Kick("BetaProbe".to_string())]);
        // Case-insensitive, but the canonical name is reported back.
        let outcome = execute("kick betaprobe", &ctx(&online));
        assert_eq!(outcome.effects, vec![Effect::Kick("BetaProbe".to_string())]);
        assert!(chat_text(&outcome.packets[0].bytes).contains("BetaProbe"));
        // Unknown players and a missing argument are errors.
        for command in ["kick", "kick Ghost"] {
            let outcome = execute(command, &ctx(&online));
            assert!(outcome.effects.is_empty(), "{command} must not kick");
        }
    }

    #[test]
    fn stop_requests_a_shutdown() {
        let online = online();
        let outcome = execute("stop", &ctx(&online));
        assert_eq!(outcome.effects, vec![Effect::Stop]);
        assert!(chat_text(&outcome.packets[0].bytes).contains("Stopping"));
    }

    #[test]
    fn unknown_and_empty_commands_are_reported_in_red() {
        let online = online();
        for command in ["", "   ", "fly", "gamemode3"] {
            let outcome = execute(command, &ctx(&online));
            assert!(outcome.effects.is_empty());
            let (id, body) = split(&outcome.packets[0].bytes);
            assert_eq!(id, CB_SYSTEM_CHAT);
            assert!(
                body.windows(3).any(|w| w == b"red"),
                "{command:?} should be red"
            );
        }
    }

    #[test]
    fn command_names_are_case_insensitive() {
        let online = online();
        assert_eq!(
            execute("STOP", &ctx(&online)).effects,
            vec![Effect::Stop],
            "command names ignore case"
        );
        assert!(!execute("LIST", &ctx(&online)).packets.is_empty());
    }
}
