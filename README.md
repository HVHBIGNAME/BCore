# BCore

<p align="center">
  <img src="site/assets/overview.svg" alt="BCore checkpoint: 665 passing tests, 88 native requests in 14 histories, 40 matching scored light snapshots. Alpha; full parity remains incomplete." width="100%" />
</p>

<p align="center">
  <a href="https://github.com/HVHBIGNAME/BCore/actions/workflows/ci.yml"><img src="https://github.com/HVHBIGNAME/BCore/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://HVHBIGNAME.github.io/BCore/"><img src="https://img.shields.io/badge/live-implementation_tracker-50e3c2?style=flat-square&amp;labelColor=111e31" alt="Live implementation tracker" /></a>
  <a href="docs/parity-report.md"><img src="https://img.shields.io/badge/native_target-26.1_%2F_775-73aaff?style=flat-square&amp;labelColor=111e31" alt="Native target: Minecraft 26.1 / protocol 775" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-b89bff?style=flat-square&amp;labelColor=111e31" alt="MIT license" /></a>
</p>

A native [Minecraft: Java Edition](https://www.minecraft.net) server written in Rust. Development and vanilla comparisons currently use **26.1 / protocol 775**; migration to **26.2 / 776** follows parity work. [SteelMC](https://github.com/Steel-Foundation/SteelMC) is the primary implementation reference.

BCore is an **independent implementation** (not a fork) aiming for vanilla parity while making better use of modern multi-core hardware — plus a native plugin system and a Bukkit/Spigot/Paper plugin bridge.

> **Status: alpha, vanilla parity incomplete.** Offline login, chunk streaming, chat, commands and persistence are implemented. Worldgen is the current priority. See the [implementation tracker](https://HVHBIGNAME.github.io/BCore/) and [measured parity report](docs/parity-report.md).

<p align="center">
  <a href="#generation-accuracy">Accuracy</a> ·
  <a href="#performance-comparison">Performance</a> ·
  <a href="#current-work">Progress</a> ·
  <a href="#building">Run BCore</a> ·
  <a href="docs/metrics/README.md">Measurement data</a>
</p>

## Generation accuracy

<img src="site/assets/generation-accuracy.svg" alt="Latest matched-history checks: zero state or biome differences in 88 requests; 40 scored light snapshots match. The older three-region snapshot has 96.99% exact-state matches. NBT and FULL-lifecycle differences remain." width="100%" />

**Latest checkpoint:** 88 requests across 14 matching native execution histories,
with **0 block-state, biome, source-order or scored-light differences**. That is
8,650,752 block-state observations, including air and repeated snapshots.
The [2026-10-08 evidence](docs/metrics/checkpoint-2026-10-08-optimized.json)
records the per-request results, frozen-source hashes and **665 passing workspace
tests**.

The older three-region comparison remains **286,047 / 294,912 states (96.99%)**,
768/768 terrain heights and 4,608/4,608 biome cells. It uses an older executable
and a different capture history. **Neither result means 100% worldgen completion.**
The remaining **36 NBT field differences** and **70 WG-heightmap presence differences**
are tracked with FULL conversion and other omissions in the
[detailed report](docs/parity-report.md).

### Concrete fixes

<img src="site/assets/regression-fixes.svg" alt="Before/after on identical native histories: fossil state differences 269 to zero across eight requests and 103 to zero across three FULL requests; desert-pyramid differences 7627 to zero across eight requests. Counts include repeated snapshots." width="100%" />

- **Trial chambers:** native decorated-pot NBT handoff now lets the neighbouring
  andesite stream finish. All nine before/after dependency chunks and RNG
  continuation witnesses match the native capture.
- **Jungle adjacency:** 10,212 native stair updates and 9,360 bamboo updates verify
  live neighbour reads, shape changes and scheduled-tick order.
- **Fossils and desert pyramids:** three parallel repair tracks reproduced missing
  terrain and deferred-NBT behavior. Fossils remove **269 + 103** differing block
  observations; desert pyramids remove **7,627**. The integrated histories match
  their native templates, shared RNG, geometry, chest flags and archaeology.
  [Implementation and native evidence](docs/generation-milestone-2026-10-07.md).

## Performance comparison

<img src="site/assets/performance.svg" alt="Measured BCore versus Vanilla 26.1 NOISE material-fill throughput at one, two and four workers, including both worldgen heightmaps, with identical block, ordered effect and WG-map hashes. BCore is still slower in this bounded kernel workload. See the benchmark guide for exact values, the optimization series and limitations." width="100%" />

This compares the **terrain material-fill kernel** in BCore and the original
Vanilla 26.1 JAR on the same machine, seed, eight chunks and worker budgets.
It includes aquifers, noise ore veins, fluid postprocessing requests and both
worldgen heightmaps; it excludes server startup, later generation stages,
storage, packets and gameplay. It is **not a full-server TPS ranking**.
Bars show medians; whiskers show variation between fresh processes.

At four workers this local test measures **44.14 chunks/s for BCore** and
**202.77 chunks/s for Vanilla**, i.e. a gap of about **4.6×** instead of the
roughly 29× recorded before the optimization series. Three exact-behavior passes
(aquifer/preliminary-surface reuse, cheaper internal lookup tables and seeded
noise lookups, and chunk-scoped material caches) took BCore from 4.31 to
44.14 chunks/s on the same contract while every native fingerprint stayed
identical.

[Exact timings, optimization series, hardware and reproduction commands](docs/performance.md) ·
[Raw measurements and hashes](docs/metrics/noise-fill-wg-v2-2026-10-08.json)

## Current work

<img src="site/assets/generation-pipeline.svg" alt="Runtime pipeline through SPAWN with explicit inputs and partial coverage. FULL conversion, live clock hookup and remaining terrain families are incomplete." width="100%" />

| Delivered | Current focus | Following work |
|---|---|---|
| Shared cross-chunk generation; native light storage and updates | Ticket-driven FULL conversion, remaining terrain families and broader seed samples | Worldgen-heightmap retirement, saved-holder hydration, Nether/End |
| Fossils; villages, ancient cities, trail/trial jigsaws; treasure, huts, jungle, desert pyramids and **desert wells** | Preserve real native behavior across combined request histories | Remaining structure families, persistent generation graph |
| Pending/materialized typed NBT; `.bcc` v5; retained SPAWN and 456 native mob handoffs | Finish live clock/settings integration and FULL boundaries | Broader performance work on the remaining ~4.6× kernel gap |

Project updates and evidence are published together in the
[tracker](https://HVHBIGNAME.github.io/BCore/), [parity report](docs/parity-report.md)
and [roadmap](docs/roadmap.md).

## Highlights

- Native Rust, multi-crate workspace (bounded contexts).
- Protocol 775: handshake, status, offline login, configuration and play.
- Native plugin system: trait-based `Plugin`, thread-safe `PluginManager`, events, dynamic `.dll`/`.so` loading.
- Data-driven seed-based Overworld generation with bundled assets and offline vanilla-reference tests. Trees, caves, ores and structures are not yet block-for-block compatible.
- Shared, status-aware generation across chunk requests, with explicit source coverage and persisted generated block-entity data, ticks and postprocessing marks.
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
cargo test --workspace --release --locked --no-fail-fast -j 1 -- --test-threads=2
```

Run the server:

```bash
cargo run -p bcore -- --host 0.0.0.0 --port 25565
```

Use a 26.1 client for the current development protocol. Generated chunks are saved
under `world/`; existing saves preserve the output of the generator that created
them. The worldgen data ships inside the executable. `BCORE_DATAPACK` optionally
selects an external extracted datapack for development.

### Generate with explicit coverage

The worldgen API retains neighbouring work across requests and reports missing
stages alongside the generated chunk:

```rust
use bcore_core::ChunkPos;
use bcore_worldgen::{generation::ChunkStatus, GenerationWorld};

let world = GenerationWorld::new(846692123413862008);
let result = world.generate_to_status(ChunkPos::new(14, 8), ChunkStatus::Light)?;

println!("Block states: {}", result.chunk.states().len());
println!("Complete native coverage: {}", result.coverage.is_complete());
for missing in &result.coverage.missing_stages {
    println!("{}: {}", missing.status, missing.reason);
}
```

Charts are committed SVGs generated from versioned evidence. To check them:

```bash
python scripts/render_readme.py --check
```

## Documentation

- [Implementation tracker](https://HVHBIGNAME.github.io/BCore/)
- [Measured parity report](docs/parity-report.md)
- [Performance comparison and methodology](docs/performance.md)
- [Published measurement data](docs/metrics/README.md)
- [Worldgen parity plan](docs/worldgen-parity-plan.md)
- [Native feature scheduling](docs/feature-scheduling.md)
- [Generated tree effects, persistence and tick preparation](docs/tree-effects.md)
- [Architecture](docs/architecture.md)
- [Reference projects](docs/references.md)
- [Paper/Purpur patch strategy](docs/paper-purpur-patches.md)
- [ADR-0001: plugin translation strategy](docs/adr/0001-plugin-translation-strategy.md)

## License

MIT — see [LICENSE](LICENSE).
