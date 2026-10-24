# Proto and level block-entity save envelopes — 2026-10-16

The first schema-3 BCore replay found extra `keepPacked` fields on proto saves.
The native capture had no such field on either the live chest or pending DUMMY
tags. Logical BE payloads, saved tick lists and map data already agreed.

## Pinned bytecode

The existing queued `javap.py` helper disassembled three classes from the pinned
26.1 server JAR. It did not initialize Minecraft classes or start a server.

- Queue: `proto-save-bytecode-20261016-01-1792121029910995700`, exit 0.
- Evidence: `target/proto-save-bytecode-20261016-01/provenance.json` and the
  corresponding `*.javap.txt` files.
- Server SHA-256:
  `a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52`.

| Class | Class-byte SHA-256 |
|---|---|
| `ProtoChunk` | `4b9006c2e449ab277587acd3611c18836270ea1a1c616ca1a2832e14c34a4227` |
| `LevelChunk` | `e7b3da1ee516413d2c8bd3c9e2f04a201d434e05335c858eb5d7dc1eb50f7683` |
| `storage.SerializableChunkData` | `440b20c0e2c45744c4c4748dce5c50568e9922833937ad198f3bcf395aec22f6` |

`ProtoChunk.getBlockEntityNbtForSaving` (bytecode offsets 0–29) first checks the
live entity. If present, it returns `saveWithFullMetadata(provider)` directly.
Otherwise it returns the existing pending-map tag directly. Neither branch
adds `keepPacked`, copies a pending tag into a new envelope, or materializes it.
An already-present field in a pending tag is therefore preserved.

`LevelChunk.getBlockEntityNbtForSaving` (offsets 0–80) invokes the level's live
lookup. A nonremoved live entity is saved with `keepPacked=false` (30–41).
If a pending tag remains, the fallback copies it and adds `keepPacked=true`
(42–80). That fallback does not modify the original pending tag.

`SerializableChunkData.copyOf` (252–333) visits the union of BE positions,
calls the selected chunk object's virtual save method, and appends each
nonnull result. It does not impose a common proto/level envelope.

## BCore correction and regression boundary

- Live payload construction is shared; only the level-save helper adds the
  level envelope.
- `ReturnedChunk::prepare_save` preserves pending proto tags verbatim and gives
  live entities precedence. A cached OLD Proto remains on this path after FULL.
- Canonical proto watches no longer invent packing fields in the diagnostic.
- The runtime regression checks absent envelopes on live and pending entities,
  retention of an existing pending field and signed-i64 payload across repeated
  saves, and live-over-pending precedence. The conversion regression contrasts
  old-proto tags with the level's materialized `keepPacked=false` tags.

The initial failed packing observations remain in
`target/native-request-replay-n1-20261016-01/comparison.json` and
`docs/metrics/native-request-initial-2026-10-16.json`. Corrected production
replays use new output directories. The comparator still scores packing fields
and retains LIGHT/SPAWN status differences; no expected native data was edited.
