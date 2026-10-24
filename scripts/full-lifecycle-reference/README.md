# Native FULL lifecycle reference

This probe invokes Minecraft **26.1 / protocol 775**'s original FULL
`ChunkStep.apply` on a small, explicitly constructed SPAWN holder in a real
pre-spawn ServerLevel. It uses the original WorldGenContext and main-thread
executor. Expected results come from the pinned native JAR, not BCore.

- Server JAR SHA-256:
  `a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`
- ASM 9.8 SHA-256:
  `876eab6a83daecad5ca67eb9fcabb063c97b5aeb8cf1fca7a989ecde17522051`
- Matched observations SHA-256:
  `07be9301756ca9fcee95012be20018f4fc2969ae1a66bba782dec1ee7b657be7`
- Evidence: [`fixtures/full-lifecycle-26_1-v1.zip`](fixtures/full-lifecycle-26_1-v1.zip).
- Archive SHA-256:
  `499950d70f9422e04c45496dfb490b9be21496d1a5cdeb3cea3512e21c090637`

Two independent server processes produced identical typed observations with
zero gameplay ticks. The synthetic holder isolates lifecycle semantics; it does
not exercise natural terrain generation or ChunkMap ticket scheduling.

## Three separate boundaries

**FULL conversion** constructs a LevelChunk, copies only the runtime heightmaps,
replaces older holder futures with `ImposterProtoChunk(allowWrites=false)`, runs
the post-load entity processor, registers already-live block entities and tick
containers, and installs the full-status supplier and unsaved listener. Existing
section and live block-entity identities survive. Pending BE tags remain pending.

**Serialization** is observable: `SerializableChunkData.copyOf` uses the native
LevelChunk BE-saving lookup, which materializes pending tags. The probe observes
maps before serialization so that this effect is distinguished from conversion.

**Ticking readiness** is later. `ChunkMap.prepareTickingChunk` invokes
`postProcessGeneration`, including fluid/block callbacks and neighbour-shape
updates. FULL itself does not consume the retained postprocessing marks.
Likewise, registering a tick container is distinct from unpacking its saved
ticks or executing them. This probe explicitly tests `unpackTicks(0)` without
running gameplay ticks or postprocessing.

The older seven-request LIGHT history
`light-history-native-1790952504492756600` illustrates the distinction: nine
DUMMY sculk tags acquire exactly 36 payload-field changes after native FULL
serialization. All 1,847 marks and 616 saved block ticks survive that transition.
This reference documents the missing lifecycle; it does **not** close BCore's
36 BE-field / 70 WG-presence differences or establish complete generation parity.

## Eleven checks

`verify.py` checks the independent captures and these native relationships:

1. FULL constructs a LevelChunk with persisted status FULL.
2. Conversion retains pending tags until save.
3. Level serialization materializes those tags.
4. The new chunk filters WG maps; the retained old proto still has them.
5. FULL, save and tick unpacking retain both duplicate marks.
6. FULL registers both block and fluid tick containers in the real level.
7. Conversion carries two deduplicated saved block ticks and one fluid tick;
   neither container's registration executes a tick.
8. Holder replacement shares sections and the already-live block entity.
9. Lower-status writes are blocked and WG height queries use runtime maps.
10. Repeated FULL preserves identity and observed effects.
11. Tick unpacking activates the saved queue without executing it.

The verifier also checks raw file manifests, frozen source/runner hashes,
matching native dependencies/settings, identical typed observations and zero
gameplay ticks before creating a new archive.

## Reproduce on the Windows worker

Prerequisites: the read-only `target/vanilla-775` and `target/jre25` caches,
`javac` on PATH, Python, and a local ASM 9.8 JAR matching the digest above.
The published captures used Temurin JRE 25.0.4.1+1 and javac 21.0.11. The runner
reuses `block-entity-lifecycle-reference/LifecycleAgent.java` and
`native-generation-reference/NativeAccess.java`; it records their source hashes.
If `target/` was cleared, restore those caches and a digest-matching ASM JAR
first. The commands below use `target/full-lifecycle-deps/asm-9.8.jar` for ASM.

Use the shared heavy-work queue, including for verification. Every capture
directory and archive must be new. Run each step only after the preceding job's
durable JSON status reports `finished` with `exit_code: 0`.

```powershell
$repo = 'C:/coding/MINECRAFT/BCore'
$queue = "$repo/scripts/run_generation_check.py"
$probe = "$repo/scripts/full-lifecycle-reference"
$asm = "$repo/target/full-lifecycle-deps/asm-9.8.jar"

python $queue --background --workdir $repo full-recapture-first -- python "$probe/capture.py" --output "$repo/target/full-lifecycle-first-new" --asm $asm

# After the first job passes:
python $queue --background --workdir $repo full-recapture-repeat -- python "$probe/capture.py" --output "$repo/target/full-lifecycle-repeat-new" --repeat-of "$repo/target/full-lifecycle-first-new" --asm $asm

# After the independent repeat passes:
python $queue --background --workdir $repo full-recapture-verify -- python "$probe/verify.py" "$repo/target/full-lifecycle-first-new" "$repo/target/full-lifecycle-repeat-new" --archive "$repo/target/full-lifecycle-recapture.zip"
```

The queue uses the main repository as cwd via `--workdir $repo`. Its background
receipt gives the durable status and child-log paths in `target/generation-checks/`;
background runner output is retained there as `<job>.runner.log`. Do not launch
competing JVM/Cargo jobs outside the queue. Retain failed attempts.

The archive contains `verify.py`, `verification.json`, and each run's original
raw/decoded observations, logs, manifests, provenance, and frozen Java/Python
sources. Hashes describe **raw bytes**: preserve Windows line endings where
present, and do not normalize archived JSON/log/source files. A new archive can
have a different ZIP digest because timestamps and provenance paths differ;
the independent decoded observations must still match exactly.

To recheck the published fixture without starting a JVM, extract its ZIP into a
fresh `target/` directory, preserving member bytes, then run the queued verifier
with the extracted `first` and `repeat` directories and a never-used output ZIP:

```powershell
python $queue --background --workdir $repo full-fixture-verify -- python "$probe/verify.py" "$repo/target/full-lifecycle-extracted-new/first" "$repo/target/full-lifecycle-extracted-new/repeat" --archive "$repo/target/full-lifecycle-reverified-new.zip"
```

## Production blockers

- A converted-holder state with read-only lower-status access and WG query
  redirection, distinct from requested status and implementation coverage.
- Saved/registered/unpacked tick containers distinct from the original ordered,
  duplicate-preserving TickRequest history; native proto scheduling saves delay
  zero and deduplicates by position/type.
- Admission and main-thread ordering for implicit ticket-level-33 FULL work
  during LIGHT requests. Converting only on explicit FULL misses that history;
  converting at every LIGHT moves the observable boundary.
- Post-load entity integration, BE registration and lookup/save boundaries, plus
  an agreed retained-world/persistence representation preserving BCC v5's typed
  pending/materialized distinction.

Postprocessing remains additional work at ticking readiness. Earlier unsupported
features, structures and entity factories must continue to report Partial even
after a bounded object conversion is implemented. Removing WG maps or eagerly
loading BEs solely because FULL was requested would not implement these semantics.
