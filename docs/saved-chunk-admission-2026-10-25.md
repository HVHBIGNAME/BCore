# Canonical saved-chunk admission — 25 октября 2026

Валидные **BCC v6 / Minecraft 26.2 / protocol 776 / data version 4903** теперь
восстанавливаются в canonical `GenerationWorld` region и `LevelChunkState`.
Streaming, generation-only observations и native requests читают общие live данные.
Задача и критерии: [#1](https://github.com/HVHBIGNAME/BCore/issues/1).

## Данные и ownership

- Storage проверяет world seed/version/registry, checksum и BCC geometry до admission.
  `SavedChunkData` дополнительно проверяет IDs и владельцев BE/effect данных.
- Восстанавливаются blocks, все 3D quart biomes, core/feature/pending BEs, generated
  и structure entities, structures, ordered tick requests, duplicate postprocessing
  и stored light. Порядок, signed delays и typed payloads сохраняются.
- Batch admission валидирует все snapshots и lighting inputs перед изменением
  canonical region. Corrupt сосед прекращает loading/generation до частичного импорта.
- Repeated/concurrent loads не заменяют уже admitted live объект. Начатая fresh
  generation claim конфликтует с поздним disk snapshot и не перезаписывает файл.
- Перед fresh generation загружается saved часть dependency envelope. Сохранённые
  Level данные доступны источникам через read-only lower-status views и не проходят
  повторную generation/decorating.

## Futures и свет

`World::request_native_chunk` публикует admitted данные через существующий request
runtime. FULL возвращает Level, lower statuses — ImposterProto, разделяющий ту же
canonical storage. Повторные запросы сохраняют object identity своего вида.
`create=false` читает saved данные после admission; он сам не запускает disk loading.

Admission завершает ранее выданные pending futures до замены holder slots,
сохраняет уже завершённые outcomes, отменяет obsolete generation/conversion demand
и обновляет retained FULL-accessibility dependencies. Four-slot negative cache
инвалидируется. Проверено завершение старых `join()` после shutdown.

Stored light и block inputs входят в общий `GenerationLight`: присутствие layers,
lazy-empty flags и nibble order сохраняются. Свет saved источника передаётся свежему
соседу; glowstone `(16,319,8)` даёт block light **14** в `(15,319,8)`.
При этом saved snapshot и disk bytes не заменяются результатом fresh lighting.
Отсутствующий sky layer читает наследуемую верхнюю плоскость; более слабая входящая
волна не должна материализовать вместо неё более тёмный layer.
Если stored light отсутствует, импортируются только block inputs без выдуманных
lighting layers или enabled-source claim.

## Coverage и persistence

BCC v6 хранит column data, а не complete native generation history. Импортированные
стадии имеют **Partial data, zero attempts**. `incoming_sources_finished` требует
реальной завершённой FEATURES attempt; saved-only 3×3 окружение оставляет его false.
WG maps, surface-biome aliases и pre-feature terrain heights не синтезируются.
Runtime heightmaps вычисляются по restored blocks.

Loaded block/fluid tick queues остаются pending до явного unpack с world game time.
Поддерживаемый chest unpack меняет canonical inventory; explicit save и restart
сохраняют результат. Это не реализация container menus/click transactions.
Native ticket expiry, eviction/unload/reload graph, world registration, полная
physics/gameplay и GUI остаются следующими границами ТЗ.

## Проверки

Общий release checkpoint: **903 passed / 0 failed / 4 ignored**, **70 suites**.
Прошли formatting, workspace build/tests и standalone replay bins. **501 source
input** был проверен до/после: `source_unchanged=true`.
[Метрика и SHA-256 логов](metrics/saved-chunk-admission-validation-2026-10-25.json).

13 `saved_admission` integration tests покрывают whole-storage equality,
negative native cache, pending futures, saved-neighbour hydration, corrupt/failed
admission, concurrency, negative coordinates, chest restart, ordered tick delays,
malformed batches/conflicts, late snapshots, boundary light и честную history flag.
Отдельные unit regressions проверяют retained FULL/accessibility и sparse light.

Воспроизведение целевого suite через ROOT queue:

```powershell
python -B scripts/run_generation_check.py saved-admission-local -- cargo test -p bcore-protocol --release --locked -j 1 --test saved_admission -- --test-threads=2
```

Новый checkpoint не меняет исторические migration/native captures и NOISE baseline.
Связанная следующая задача: [authoritative creative break/place #3](https://github.com/HVHBIGNAME/BCore/issues/3).
