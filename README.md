# BCore

A native [Minecraft: Java Edition](https://www.minecraft.net) server written in Rust. Development and vanilla comparisons currently use **26.1 / protocol 775**; migration to **26.2 / 776** follows parity work. [SteelMC](https://github.com/Steel-Foundation/SteelMC) is the primary implementation reference.

BCore is an **independent implementation** (not a fork) aiming for vanilla parity while making better use of modern multi-core hardware — plus a native plugin system and a Bukkit/Spigot/Paper plugin bridge.

> **Status: alpha, vanilla parity incomplete.** Offline login, chunk streaming, chat, commands and persistence are implemented. Worldgen is the current priority. See the [implementation tracker](https://HVHBIGNAME.github.io/BCore/) and [measured parity report](docs/parity-report.md).

## Highlights

- Native Rust, multi-crate workspace (bounded contexts).
- Protocol 775: handshake, status, offline login, configuration and play.
- Native plugin system: trait-based `Plugin`, thread-safe `PluginManager`, events, dynamic `.dll`/`.so` loading.
- Data-driven seed-based Overworld generation with bundled assets and offline vanilla-reference tests. Trees, caves, ores and structures are not yet block-for-block compatible.
- JVM virtualization bridge for Bukkit/Spigot/Paper plugins (ADR-0001) — loads an original `.jar` and invokes `onEnable`.

## Crates

| Crate | Purpose |
|---|---|
| `bcore` | Server binary |
| `bcore-core` | Shared types, version constants, protocol primitives |
| `bcore-protocol` | Protocol 775: login/play, commands, streaming and persistence |
| `bcore-plugin` | Native plugin API and manager |
| `bcore-plugin-java` | JVM bridge for legacy Java plugins |
| `bcore-worldgen` | Deterministic seed-based generation |
| `bcore-registry` | Data-driven block/item registry (future) |

## Building

```bash
cargo build --workspace
cargo test --workspace --release --locked -- --test-threads=4
```

Run the server:

```bash
cargo run -p bcore -- --host 0.0.0.0 --port 25565
```

Use a 26.1 client for the current development protocol. Generated chunks are saved
under `world/`; existing saves preserve the output of the generator that created
them. The worldgen data ships inside the executable. `BCORE_DATAPACK` optionally
selects an external extracted datapack for development.

## Documentation

- [Implementation tracker](https://HVHBIGNAME.github.io/BCore/)
- [Architecture](docs/architecture.md)
- [Reference projects](docs/references.md)
- [Paper/Purpur patch strategy](docs/paper-purpur-patches.md)
- [ADR-0001: plugin translation strategy](docs/adr/0001-plugin-translation-strategy.md)

## License

MIT — see [LICENSE](LICENSE).
