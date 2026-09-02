use std::net::TcpListener;

use bcore_core::version::{IMPLEMENTATION_NAME, MC_VERSION, PROTOCOL_VERSION};
use bcore_protocol::server;

fn main() {
    let (host, port) = parse_args();

    let listener = match TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[{IMPLEMENTATION_NAME}] failed to bind {host}:{port}: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "[{IMPLEMENTATION_NAME}] Minecraft {MC_VERSION} (protocol {PROTOCOL_VERSION}) listening on {host}:{port}"
    );
    println!(
        "[{IMPLEMENTATION_NAME}] gameplay is not implemented yet — server-list ping works, login disconnects gracefully."
    );

    server::run(listener);
}

/// Parse `--host` / `--port` command-line arguments, with defaults.
fn parse_args() -> (String, u16) {
    let mut host = "0.0.0.0".to_string();
    let mut port: u16 = 25565;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                if let Some(v) = args.get(i + 1) {
                    host = v.clone();
                    i += 1;
                }
            }
            "--port" => {
                if let Some(v) = args.get(i + 1) {
                    if let Ok(p) = v.parse() {
                        port = p;
                    }
                }
                i += 1;
            }
            "--help" | "-h" => {
                println!("usage: bcore [--host <addr>] [--port <port>]");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    (host, port)
}
