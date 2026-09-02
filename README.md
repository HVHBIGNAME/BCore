# BCore

A native [Minecraft: Java Edition](https://www.minecraft.net) server written in Rust, targeting **26.2** (protocol **776**). Inspired by [SteelMC](https://github.com/Steel-Foundation/SteelMC), [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin), [Valence](https://github.com/valence-rs/valence), [Cuberite](https://github.com/cuberite/cuberite) and [Glowstone](https://github.com/GlowstoneMC/Glowstone).

BCore is an **independent implementation** (not a fork) aiming for vanilla parity while making better use of modern multi-core hardware — plus a native plugin system and a Bukkit/Spigot/Paper plugin bridge.

> **Status: pre-alpha / proof of concept.** Server-list ping works; login and gameplay are not implemented yet. Track progress on the [implementation tracker](https://HVHBIGNAME.github.io/BCore/).

## Highlights

- Native Rust, multi-crate workspace (bounded contexts).
- Protocol 776 (26.2): handshake + server-list status ping.
- Native plugin system: trait-based `Plugin`, thread-safe `PluginManager`, events, dynamic `.dll`/`.so` loading.
- Deterministic seed-based world generation (prototype).
- JVM virtualization bridge for Bukkit/Spigot/Paper plugins (ADR-0001) — loads an original `.jar` and invokes `onEnable`.

## Crates

| Crate | Purpose |
|---|---|
| `bcore` | Server binary |
| `bcore-core` | Shared types, version constants, protocol primitives |
| `bcore-protocol` | Protocol 776: handshake/status/login, TCP server |
| `bcore-plugin` | Native plugin API and manager |
| `bcore-plugin-java` | JVM bridge for legacy Java plugins |
| `bcore-worldgen` | Deterministic seed-based generation |
| `bcore-registry` | Data-driven block/item registry (future) |

## Building

```bash
cargo build --workspace
cargo test --workspace
```

Run the server:

```bash
cargo run -p bcore -- --host 0.0.0.0 --port 25565
```

A real 26.2 client will then see the server in its server list.

## Documentation

- [Implementation tracker](https://HVHBIGNAME.github.io/BCore/)
- [Architecture](docs/architecture.md)
- [Reference projects](docs/references.md)
- [Paper/Purpur patch strategy](docs/paper-purpur-patches.md)
- [ADR-0001: plugin translation strategy](docs/adr/0001-plugin-translation-strategy.md)

## License

MIT — see [LICENSE](LICENSE).
