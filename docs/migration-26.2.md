# Minecraft Java 26.2 migration

BCore targets **Minecraft 26.2, protocol 776, data version 4903**. The authenticated
server JAR is:

```text
target/vanilla-776/versions/26.2/server-26.2.jar
183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8
```

The migration preserves the earlier 26.1 fixtures and measurement series as
explicit historical references. Native template, speleothem, tree, numeric and
loot captures have independent target identities and repeat checks.

## Verified checkpoint

| Boundary | Result |
| --- | --- |
| Workspace release tests | 886 passed, 0 failed, 4 ignored; 69 suites, including the published-bootstrap regression |
| Formatting / release build / diagnostics examples | Passed; deprecated example calls replaced with retained `GenerationWorld` and explicit coverage |
| Shared registry | 32366 states, 1196 blocks, 1537 items, 158 entity types, 40 attributes, 49 block-entity types, 66 biomes |
| Registry source regeneration | Published Rust source/provenance reproduce with recorded CRLF-to-LF normalization; native data checks remain byte-exact |
| Template runtime | All 153 configured-feature cases, 12 codec cases and 55 potent-sulfur load/reset cases pass |
| Speleothem runtime | 102 target cases and 18 codec cases; independent repeat verified |
| Numeric replay | 712 target cases / 51003 exact values, including lazy interval selection and four-worker/reverse checks |
| Loot replay | 137 target fills with typed NBT and RNG checkpoints; location-aware loaded-world context |
| Native-decoded BCore protocol | 1179 / 1179 frames, including 127 terrain packets, across three sessions on the final rebuilt executable |
| Session ownership | Same process reuses its session UUID; restart creates a different UUID |
| Shutdown / restart | Both operator `/stop` runs exit 0; owned listeners disappear and ports can be rebound exclusively |
| Persistence | Production BCC v6 codec changes state 2 to diamond-block state 5309 at `(7,0,-9)` while stopped; restarted chunk packet contains the marker and save bytes stay identical |
| 26.2 NOISE baseline | All 18 process records match blocks, ordered marks and both WG maps; BCore 47.79 / 83.98 / 156.81 chunks/s at 1/2/4 workers |

The [complete workspace report](metrics/migration-workspace-validation-2026-10-24.json)
preserves all 69 suite results and log SHA-256
`55c12384f057766dd784f9f740c34d2bee122d07f715ae5a4023c4675cc28776`.

The saved edit is an isolated persistence test through `ChunkStore`, rather than a
claim that player block interaction is implemented. The socket probe uses the real
26.2 registry-aware packet codecs. GUI rendering, online authentication, container
menus and complete native FULL/gameplay behavior remain unverified or incomplete.
Fresh generation still reports Partial coverage, including missing stage work.

## Registry and bootstrap ownership

The registry fingerprint is:

```text
1affd10b6938b00feda7fb9e22a00c5ff5b76a34446c6176059ade9ac22264ef
```

All 66 network biome IDs match the shared registry. Native synchronized dimension
types are `overworld=0`, `overworld_caves=1`, `the_end=2`, `the_nether=3`.

The preserved embedded configuration has the same ordered registry entries,
tags and typed NBT as two independently authenticated 26.2 captures. NBT compound
member ordering changes raw hashes, so raw equality and typed-value equality are
reported separately; list order, IDs, numeric widths and floating-point bits are
preserved. The existing play replay and owner-generated packets pass native decoding.

`crates/bcore-protocol/data/protocol_26_2.json` binds both embedded packet blobs,
official capture manifests and the earlier 1249-frame frozen-executable proof.
The newer [versioned protocol validation](metrics/protocol-26_2-validation-2026-10-24.json)
binds the same bytes and both official captures to the final rebuilt executable,
1179 native-decoded frames and the full restart/persistence proof. The original
publication remains unchanged. `generated_source_26_2.json` supplements the registry publication
without replacing its native capture/data digests.

## Run a fresh world

Build through the repository's single heavy-work coordinator:

```powershell
python -B scripts/run_generation_check.py local-release -- cargo build --workspace --release --locked -j 1
```

Start the resulting server:

```powershell
.\target\build\release\bcore.exe --world world-26.2 --seed 846692123413862008 --host 127.0.0.1 --port 25565
```

Connect a Java **26.2** client to **127.0.0.1:25565**. This is offline login; console
`op PlayerName` grants operator status in the selected world. `stop` in the console
or operator `/stop` ends the server cleanly.

The tested frozen executable is also available locally:

```powershell
.\target\migration-26_2-release-freeze-20261024-03\bin\bcore.exe --world world-26.2 --seed 846692123413862008 --host 127.0.0.1 --port 25565
```

Its SHA-256 is
`4e6cb4e410d241d8e40c36362a5faa65a21f1418380a6f61717f28bd5bf38400`.
The frozen source archive is
`9c371cc33c871a23fffae5c8895171f33a63c1169848c8e300fdfaecc48aa0dd`
with 489 source inputs. This is a post-build executable/source freeze, linked to the
retained release build and full workspace validation; the NOISE benchmark separately
builds its own archived source tree.

## Save admission

`bcore-world.json` stores the selected seed, protocol/data version and registry
fingerprint. Operators and bans belong to the same root. BCC **v6** stores the
registry identity in its 64-byte header, with typed NBT, queued effects and checksum.
Pre-v6, unidentified, foreign, malformed and corrupt saves fail admission rather
than being regenerated or overwritten. There is no automatic old-world conversion.

## Reproduce protocol and durability checks

The driver takes already-built executables, starts owned processes on exclusively
reserved loopback ports, and records complete bounded transcripts. Every output
directory must be new.

```powershell
python -B scripts/run_generation_check.py persistence-probe-build -- cargo build -p bcore-protocol --release --locked -j 1 --example persistence_probe
python -B scripts/run_generation_check.py --background protocol-26_2-NEW -- python -B scripts/migration-26.2/protocol_capture.py bcore --exercise full --executable target/build/release/bcore.exe --persistence-probe target/build/release/examples/persistence_probe.exe --output target/protocol-26_2-NEW --seed 846692123413862008 --port 0
```

The full probe checks status/version admission, login/configuration/play, chat,
movement, ping/keepalive, reconnect UUID ownership, two clean shutdowns and the
saved marker after restart. Native decoding requires every recorded frame and all
synchronized registries; incomplete/failed validation cannot publish bootstrap data.

Retained local evidence:

- `target/migration-26_2-workspace-tests-20261023-03.json` — complete 886-test log summary.
- `target/migration-26_2-bcore-protocol-20261024-04/results.json` — full native decode and persisted-edit result on the final rebuilt executable.
- `target/migration-26_2-release-freeze-20261024-03/release.json` — frozen executable/source and build identities.
- `target/migration-26_2-protocol-publication-20261024-02.json` — verified final protocol publication/readback.
- `target/migration-26_2-protocol-bootstrap-publish-20261023-01.json` — preserved earlier bootstrap publication/readback.
- `target/migration-26_2-registry-data-20261023-07/published_verification.json` — complete registry-source regeneration.

## Performance and remaining work

The 26.2 baseline uses the unchanged `noise-fill-wg-v2` workload: seed
`846692123413862008`, eight chunks, workers **1/2/4**, three fresh processes,
ten warmup and ten measured batches. Actual native execution proves **26.2 / 776 /
4903** with all 39 authenticated libraries. Every process and its warmup/measured
batches match complete block/effect/WG-heightmap fingerprints.

| Workers | Vanilla 26.2 chunks/s | BCore chunks/s |
| --- | ---: | ---: |
| 1 | 49.77 | 47.79 |
| 2 | 74.03 | 83.98 |
| 4 | 215.35 | 156.81 |

[Versioned timings and provenance](metrics/noise-fill-wg-v2-baseline-26_2-2026-10-24.json)
retain every duration and frozen artifact identity. The host is unisolated and the
native one/two-worker processes vary substantially; this baseline establishes no
cross-version optimization gain. Historical 26.1 gains remain in
[the performance guide](performance.md). The
[fresh owned-child profile](performance-profile-26.2-2026-10-24.md) collected 4,002
leaf samples with matching kernel fingerprints; interpolation and noise/density
evaluation are the next measured optimization targets.

Remaining work includes full ticket/FULL lifecycle and saved-holder hydration,
source-owned gameplay ticks/effects, broader seeds/dimensions, block interaction,
player inventories and real menu/authentication validation. A complete Aislop scan
is unavailable in this environment; the scanner executable is absent.
