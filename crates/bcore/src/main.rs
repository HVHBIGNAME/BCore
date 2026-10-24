use std::net::TcpListener;
use std::path::PathBuf;

use bcore_core::version::{IMPLEMENTATION_NAME, MC_VERSION, PROTOCOL_VERSION};
use bcore_protocol::{server, shared::open_shared_server, world_state::DEFAULT_SEED};

const USAGE: &str =
    "usage: bcore [--host <addr>] [--port <port>] [--world <directory>] [--seed <i64>]";

fn main() {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return;
        }
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let state = match open_shared_server(&options.world, options.seed) {
        Ok(state) => state,
        Err(error) => {
            eprintln!(
                "[{IMPLEMENTATION_NAME}] cannot admit world {}: {error}",
                options.world.display()
            );
            std::process::exit(1);
        }
    };

    let listener = match TcpListener::bind((options.host.as_str(), options.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[{IMPLEMENTATION_NAME}] failed to bind {}:{}: {e}",
                options.host, options.port
            );
            std::process::exit(1);
        }
    };

    println!(
        "[{IMPLEMENTATION_NAME}] Minecraft {MC_VERSION} (protocol {PROTOCOL_VERSION}) listening on {}:{}",
        options.host, options.port
    );
    println!(
        "[{IMPLEMENTATION_NAME}] world={} seed={}",
        options.world.display(),
        options.seed
    );
    println!(
        "[{IMPLEMENTATION_NAME}] реалистичный террейн + чат + команды + сохранение мира; блоки/инвентарь/сущности — в работе."
    );

    bcore_protocol::console::start(state.clone());
    server::run_with_state(listener, state);
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    host: String,
    port: u16,
    world: PathBuf,
    seed: i64,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        host: "0.0.0.0".into(),
        port: 25565,
        world: PathBuf::from("world-26.2"),
        seed: DEFAULT_SEED,
    };
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if matches!(argument.as_str(), "--help" | "-h") {
            return Ok(None);
        }
        if !matches!(
            argument.as_str(),
            "--host" | "--port" | "--world" | "--seed"
        ) {
            return Err(format!("unknown argument: {argument}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {argument}"))?;
        if value.starts_with("--") {
            return Err(format!("missing value for {argument}"));
        }
        match argument.as_str() {
            "--host" => {
                if value.is_empty() {
                    return Err("--host must not be empty".into());
                }
                options.host = value;
            }
            "--port" => {
                options.port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            "--world" => {
                if value.is_empty() {
                    return Err("--world must not be empty".into());
                }
                options.world = PathBuf::from(value);
            }
            "--seed" => {
                options.seed = value
                    .parse()
                    .map_err(|_| format!("invalid seed: {value}"))?;
            }
            _ => unreachable!("validated option"),
        }
    }
    Ok(Some(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_a_separate_migration_world() {
        let options = parse_args(Vec::new()).unwrap().unwrap();
        assert_eq!(options.world, PathBuf::from("world-26.2"));
        assert_eq!(options.seed, DEFAULT_SEED);
    }

    #[test]
    fn explicit_world_and_signed_seed_are_parsed_without_io() {
        let args = [
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--world",
            "isolated world",
            "--seed",
            "-42",
        ];
        let options = parse_args(args.into_iter().map(String::from))
            .unwrap()
            .unwrap();
        assert_eq!(options.host, "127.0.0.1");
        assert_eq!(options.port, 0);
        assert_eq!(options.world, PathBuf::from("isolated world"));
        assert_eq!(options.seed, -42);
    }

    #[test]
    fn invalid_or_missing_arguments_are_errors() {
        for args in [
            vec!["--seed"],
            vec!["--port", "65536"],
            vec!["--seed", "no"],
            vec!["--world", "--seed"],
            vec!["--other"],
        ] {
            assert!(parse_args(args.into_iter().map(String::from)).is_err());
        }
    }
}
