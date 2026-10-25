# Authoritative creative block gameplay — 25 октября 2026

Задача [#3](https://github.com/HVHBIGNAME/BCore/issues/3) реализует первый
проверяемый gameplay slice для **Minecraft 26.2 / protocol 776 / data version 4903**:
обычные stateless full-cube блоки, creative hotbar и реальные ломка/установка.

## Реализовано

- Connection parser проверяет packet layout, enum/slot/item/count bounds, boolean
  fields и отсутствие trailing bytes. Неподдерживаемые component patches и malformed
  payloads закрывают connection до изменения мира.
- Bounded очередь передаёт действия единственному владельцу на `bcore-tick`.
  Token принадлежит конкретному `World` и thread; его нельзя передавать между threads.
- Gameplay читает только уже доступный canonical Level. Unloaded targets не
  запускают generation. Проверяются `player_loaded`, teleport confirmation,
  creative mode, main hand, supported item/state, world height/border и reach.
- `START_DESTROY_BLOCK` выполняет instant creative destroy. `ABORT`/`STOP` не
  уничтожают новый блок; другие action kinds в этом slice не меняют world state.
- Placement использует выбранный hotbar slot, native BlockItem → state binding,
  basic clicked block и air-only соседнюю позицию. Standing-player overlap
  отвергается для отправителя и остальных отслеживаемых игроков.
- Compare/write меняет canonical states и runtime heightmaps; live/pending BEs,
  structured entities, ordered ticks и generation coverage не заменяются snapshot.
- Изменения инвалидируют encoded payloads. Snapshot и subscription нового viewer
  публикуются под одним read lock до socket write; последующая correction не
  теряется во время первой отправки terrain. Socket I/O не удерживает world lock.
- Block updates идут только viewers нужного чанка; connection самостоятельно
  отправляет свой outbox. Teleport сразу удаляет старую subscription.
- Shared incremental light updates публикуются для изменённых Level columns.
  Native stored layer presence/empty flags сохраняются; absent light не выдаётся
  за initialized source. Полный Phase 4 остаётся Partial.
- Dirty BCC save сохраняет inert canonical data без постороннего BE lookup или
  tick unpack. Failed save сохраняет original file и dirty marker для retry.
  Shutdown отменяет queued requests и joins окончательные dirty saves.
- Sequence acknowledgements coalesce по maximum sequence на tick и отправляются
  после authoritative обработки и успешного dirty-save boundary.

## Native wire authority

Два независимых `STREAM_CODEC` runs совпали на **25 случаях**. Проверены восемь
action ordinals (включая 26.2 `STAB`), шесть use-item faces, carried/creative slots,
block ack/update и inventory corrections. Decode поглотил все bytes.

| Packet | Native ID |
|---|---:|
| serverbound player_action | 41 |
| serverbound set_carried_item | 53 |
| serverbound set_creative_mode_slot | 56 |
| serverbound use_item_on | 66 |
| clientbound block_changed_ack | 4 |
| clientbound block_update | 8 |
| clientbound light_update | 48 |
| clientbound set_player_inventory | 108 |

Use-item hit имеет отдельные `inside` и `world_border` flags. Position packing,
action byte face / use-item VarInt face, float offsets и unpatched optional stack
проверены отрицательными coordinates и strict malformed/truncation tests.
Light-update ID закреплён protocol inventory; 25-case codec fixture не декодирует
сам light payload — здесь используется ранее проверенный shared chunk encoder.

Native BlockItem inventory содержит **197** bindings: exact `BlockItem`, block
class `Block`/`DropExperienceBlock`, no state properties, no BE/fluid, full collision
cube. Это bounded catalog, а не 197 независимых runtime gameplay tests.
Standing dimensions/eye height сохранены как exact native float-to-double values.
Block range: default **4.5**, creative addition **0.5**, handler tolerance **1.0**;
проверяется strict eye-to-block AABB distance.

JAR SHA-256:
`183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8`.
[Fixture](../crates/bcore-protocol/data/block_actions_26_2.json) и
[provenance](../crates/bcore-protocol/data/block_actions_provenance_26_2.json)
содержат source/Java/library identities и repeat hashes. Fixtures не нормализуются
Git и не заменяют historical captures.

## Проверки

Focused checkpoint: **37 passed** — 7 native-wire tests, 5 mutation/ownership/save
tests, 6 actual TCP gameplay scenarios и 19 прежних join/chat regressions.
Workspace release: **921 passed / 0 failed / 4 ignored**, **71 suites**; formatting,
build и standalone replay прошли, **517 source inputs unchanged**. Четыре native
helper sources, JAR, Java runtime и 39 libraries дополнительно сверены с provenance.
[Результаты и SHA-256](metrics/creative-block-gameplay-validation-2026-10-25.json).

TCP tests упражняют actual requests, second viewer, chunk-border placement,
negative coordinates, mode/reach/hand/hit/unloaded/BE rejection, malformed
connection close, canonical retained read, dirty persistence и restart/reload.
Barrier test отдельно воспроизводит edit во время first terrain write.

```powershell
python -B scripts/run_generation_check.py block-gameplay-local -- cargo test -p bcore-protocol --release --locked -j 1 --test block_actions -- --test-threads=2
python -B scripts/run_generation_check.py block-native-capture -- python -B scripts/migration-26.2/block_actions.py --output target/block-native-local
```

## Открытые границы ТЗ

Полные physics/collision, crouch/swim pose handling, survival inventory, drops,
menus/click transactions, blocks со state properties и neighbour callbacks,
containers/BEs, redstone/fluids и полный ticket/light lifecycle остаются отдельными
этапами. Creative hotbar ограничен slots **36..44 / hotbar 0..8**, main hand и
unpatched stacks; session inventory не является persisted playerdata.

Socket/native-codec checks не являются GUI-проверкой. Online auth не добавлен.
NOISE throughput не измеряется этими тестами; #2 остаётся открыт.
