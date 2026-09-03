//! stdin console for commands executed with server operator privileges.

use std::io::{self, BufRead};
use std::thread;

use crate::command::{self, CommandContext, Destination, Effect};
use crate::shared::SharedServer;

/// Start a detached stdin reader. Each non-empty line is executed as `Server`.
pub fn start(server: SharedServer) {
    thread::Builder::new()
        .name("bcore-console".into())
        .spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                execute_line(&server, &line);
                if server.is_shutting_down() {
                    break;
                }
            }
        })
        .expect("spawn console thread");
}

fn execute_line(server: &SharedServer, line: &str) {
    let command_text = line.trim().trim_start_matches('/').trim();
    if command_text.is_empty() {
        return;
    }
    let online = server.player_names();
    let ctx = CommandContext {
        sender_name: "Server",
        online: &online,
        max_players: crate::join::MAX_PLAYERS,
        seed: crate::world_state::DEFAULT_SEED,
        spawn: crate::world_state::shared().spawn_position(0.5, 0.5),
        is_op: true,
    };
    let outcome = command::execute(command_text, &ctx);
    for packet in &outcome.packets {
        match packet.destination {
            Destination::Sender | Destination::Everyone => server.broadcast(&packet.bytes),
            Destination::Others => server.broadcast(&packet.bytes),
        }
    }
    for effect in &outcome.effects {
        match effect {
            Effect::Kick(name) => {
                if let Some(player) = server.find_by_name(name) {
                    player.kick();
                }
            }
            Effect::Stop => server.request_shutdown(),
            Effect::SetOp { name, op } => {
                if *op {
                    server.add_op(name);
                } else {
                    server.remove_op(name);
                }
                server.save_ops();
            }
            Effect::SetDayTime(_) | Effect::SetGameMode(_) | Effect::Teleport { .. } => {
                println!("[bcore] console effect requires a player target: {effect:?}");
            }
        }
    }
    println!(
        "[bcore] > /{command_text}: packets={}, effects={}",
        outcome.packets.len(),
        outcome.effects.len()
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn slash_is_optional() {
        assert_eq!("help", "/help".trim_start_matches('/'));
    }
}
