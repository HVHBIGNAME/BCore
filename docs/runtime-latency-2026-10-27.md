# Runtime: задержки действий и выдача чанков — 27 октября 2026

Исправления отвечают на ручной тест, в котором генерация задерживала игровой тик,
быстрые ломания подтверждались последовательно, а установка на траву отклонялась.
Цель этого checkpoint — отзывчивость **BCore 26.2 / protocol 776 / data version 4903**.
[Проверяемые метрики](metrics/runtime-latency-validation-2026-10-27.json) и
[acceptance #6](https://github.com/HVHBIGNAME/BCore/issues/6) связывают результаты с
исходниками, исполняемыми файлами и журналами.

## Результат

- Полный release workspace: **939 passed / 0 failed / 4 ignored**, **72 suites**;
  868 входных файлов не изменились во время проверки. Проверены также standalone
  replay, семь Python suites и семь SVG. Все **4592** файла пользовательских миров
  и операторов сохранили bytes и membership.
- Серия из 32 настоящих TCP creative breaks: медиана времени до durability ack
  **1881,04 → 243,79 ms**, примерно **7,72×** относительно предыдущего BCore.
  Во всех повторах второй клиент получил обновления всех 32 блоков; чтение BCC подтвердило AIR.
- Две пары production-процессов при активной генерации: p95 correction/ack
  **1718/1781 → 141/187 ms**. Новая сборка показала **20,0 TPS во всех 11 окнах**.
- Отдельная 30-секундная сессия выполнила **488 реальных изменений** — 244 установки
  и 244 ломания — одновременно с выдачей **171 чанка**. Порядок состояний совпал,
  все действия подтверждены, после shutdown в BCC остался AIR; все 6 окон — 20,0 TPS.

Это сравнение предыдущей и исправленной сборок BCore. Общая цель превосходства над
Java остаётся в [#2](https://github.com/HVHBIGNAME/BCore/issues/2); визуальная
плавность GUI-клиента требует отдельного ручного теста.

## Измеренные нагрузки и компромиссы

Хост: Windows 11, Ryzen 7 5700X (8 cores / 16 logical), без OS isolation.
Процессы запускались последовательно через ROOT queue с `RAYON_NUM_THREADS=2`,
обычным release profile и закреплёнными исходниками. Замеры выполнены 26 октября,
полная повторная workspace-проверка закончилась 27 октября.

### Быстрая серия ломаний

Это release integration-test server с сохранённой fixture, actor и viewer.
`Instant` измеряет отправку 32 действий и ожидание последнего ack; viewer updates и
saved readback проверяются после остановки таймера. Отдельные production `.exe`
используются в следующих двух нагрузках.

| Повтор | Старый BCore, ms | Исправленный, ms | Ack packets, до → после | Light packets, до → после |
|---|---:|---:|---:|---:|
| 1 | 1881,04 | 243,79 | 32 → 3 | 94 → 20 |
| 2 | 1712,26 | 494,23 | 32 → 4 | 94 → 20 |
| 3 | 2041,18 | 182,26 | 32 → 3 | 94 → 17 |

### Production generation + correction/ping

В каждом свежем процессе копируются metadata и одинаковые 3×3 saved columns;
остальные чанки генерируются. Seed `846692123413862008`; 30 секунд после первого
terrain packet; клиент запрашивает 1 чанк/tick. Каждые 200 ms он отправляет ABORT
действие и ping. ABORT проверяет authoritative correction/ack без изменения блока.
Порядок процессов: **old02 → new02 → new03 → old03**.

| Run | Correction median / p95 / max, ms | Первый чанк, s | Выдано чанков | TPS окна |
|---|---:|---:|---:|---|
| old02 | 93 / 1718 / 2328 | 1,984 | 320 | 4,9 / 11,8 / 13,7 / 15,2 / 17,5 |
| new02 | 125 / 141 / 406 | 2,688 | 245 | 20 во всех 6 окнах |
| new03 | 125 / 187 / 390 | 1,625 | 238 | 20 во всех 5 окнах |
| old03 | 93 / 1781 / 2391 | 1,797 | 318 | 4,9 / 11,5 / 15,7 / 15,3 / 17,1 |

Длинные задержки уменьшились, но медиана correction выросла **93 → 125 ms**.
За одинаковое окно доставлено меньше чанков: новый sender учитывает client credit
и outstanding ack, вместо отправки batches по частоте входящих packets. Поэтому
эти результаты подтверждают отзывчивость и pacing, а не выигрыш общего generation
throughput. Максимальный batch был 8 до и 4 после (initial batch); последующий request
1.0 обслуживается по одному чанку.

Сравнены hashes полных terrain payloads общих колонок: **245 + 238 = 483 пары,
0 различий**. Повторы могут содержать одни и те же координаты; 483 — число парных
сравнений. Это bounded before/after equality, не новая проверка всей генерации
против JVM.

Ping p95 после исправления оказался ниже разрешения `time.monotonic()`;
максимум во втором новом run — 313 ms. Нулевые samples не означают нулевую latency.
Первый terrain packet варьируется между запусками; startup gain не заявляется.

### Production generation + реальные edits

Nominal interval 50 ms, 30 секунд, `time.perf_counter()`. Реально отправлены 488 действий,
а не гарантированные 600: scheduling клиента и обработка пакетов входят в нагрузку.
Exact update sequence `[stone, AIR]` повторилась 244 раза; BCC readback после clean
shutdown проверил последнее состояние. Ack latency: **median 124,31 ms,
p95 187,50 ms, max 614,18 ms**.

Максимальное время gameplay work по 6 reporting windows:
**24,86 / 1,98 / 0,98 / 1,63 / 2,32 / 1,46 ms**. 10 ms budget проверяется между действиями;
одна операция или snapshot может превысить его. TPS и время выполнения работы
показываются отдельно в серверном журнале.

## Изменения реализации

1. **Приём действий:** bounded FIFO принимает packets без ожидания игрового тика.
   Connection polls completion и продолжает читать movement/ping. Player context
   фиксируется при enqueue; остаются owner, ready, mode, reach и disconnect checks.
   Global channel содержит 128 элементов плюс один deferred request у owner;
   connection-local completions также ограничены 128.
2. **Частичные TCP frames:** `FrameReader` сохраняет length/ID/body через timeout,
   проверяет malformed/oversized frames и корректно читает следующий пакет.
3. **Generation handoff:** отдельный generation-order mutex сохраняет порядок
   source stages; state mutex освобождается между ними. Tick получает loaded-world
   guard через `try_lock`, либо переносит FIFO work на следующий tick. Waiting tick
   получает ограниченное 50 ms окно handoff, не останавливающее generation навсегда.
4. **Batch publication:** mutations, corrections, invalidation и snapshot выполняются
   в одной loaded-world/publication boundary; batch имеет лимит 128 действий и 10 ms
   inter-action budget. Socket writes остаются у connection.
5. **Saving:** один background writer выполняет encoding, file validation, atomic
   save и fsync. Completion содержит captured ack и dirty revisions. Старый write
   очищает только свою revision; более новые edits остаются dirty. Ack публикуется
   только после успешной записи; shutdown ждёт writer и flush оставшихся изменений.
   Save snapshots инертны для pending block entities и ordered ticks.
6. **Lighting:** canonical block сверяется с уже зарегистрированным light input.
   Полная колонка повторно регистрируется только при отсутствии input. Публикуются
   только изменившиеся light columns, coalesced за batch; encoder заимствует light
   arrays без копирования terrain/biomes.
7. **Chunk pacing:** 50 ms cadence, fractional credit и один outstanding batch;
   ack клиента регулирует следующие выдачи. Flood movement packets не ускоряет
   sender, unsolicited ack не увеличивает credit.
8. **Grass surfaces:** native registry/predicate checks допускают `grass_block`,
   `podzol`, `mycelium` как support и creative-break states. Placement catalogue
   остаётся 253 full-cube items; новые item placement families не добавлены.

## Проверки и границы

Добавлены 12 регрессий: fragmented TCP + следующий ping, grass support/break,
ordered 32-action burst, bounded cancellation, частичный framing, client credit,
clean tick при publication reader, busy generation, blocked writer и old-save/new-edit
revision с сохранением pending NBT. Полные имена находятся в метрике.

Focused run: 196 protocol unit + 9 block-action TCP + 3 pillar TCP + 13 chat + 6 join
+ 118 generation tests, затем 3 отдельных burst processes. Полный workspace содержит
**939 тестов**, включая эти регрессии. Python suites: 29 export, 3 timing, 4 renderer,
8 queue, 21 benchmark, 43 replay, 116 migration. Все команды, log hashes, raw-result
hashes и входной manifest сохранены.

Исходный grass regression завершался `exit 101`; это ожидаемое воспроизведение
дефекта. Первый full workspace attempt прерван Windows с `0x40010004` и исключён
из successful evidence. Новый полный run прошёл все 12 проверок. Локальный `aislop`
не найден; full scan/score отсутствует.

Прежние native placement/codec fixtures и пять helper sources сохранены byte-exact;
новый JVM capture в этом checkpoint не выполнялся. FULL lifecycle, broader world
parity, poses/physics, survival/drops, остальные stateful blocks, menus/playerdata
и online authentication остаются в [общем ТЗ](roadmap.md).

## Воспроизведение и артефакты

Функциональная TCP-регрессия из checkout:

```text
python -B scripts/run_generation_check.py runtime-burst-check -- cargo test -p bcore-protocol --release --locked -j 1 --test block_actions rapid_creative_break_burst_is_ordered_published_and_durable -- --exact --nocapture --test-threads=1
```

Для полного checkpoint используется workspace command из метрики. Public report
содержит derived observations; raw results/logs, frozen binaries, fixture copies и
probe drivers находятся в локальных `target/runtime-*` roots. Повтор production
нагрузок требует тех же hash-identified saved 3×3 inputs и новых output directories.

| Артефакт | SHA-256 |
|---|---|
| Old production exe (`75f21ec`) | `e28263860364f8961ea9eeaa6229e8093c1fad706bb8ef3524f3ae325fbf3ab5` |
| Measured fixed production exe | `710a540f48bc2cb155a24ec61d09c140ab6502e93f770582b618f1836b1f48c0` |
| Source archive (868 inputs) | `3d19b28e4dc1420fce6737fa76b0914a0c9e12cdeea7332a1c0e3dab304d6c83` |
| Full ROOT validation | `f8e759e77cc79d0a1bed4e534b0b23176b23a2f4bd464db25a2d820f25031262` |

Архив фиксирует 868 входов до добавления публикационной документации. Окончательное
Git tree проверяется в independent index checkout; exact-commit CI/Pages и live
readback привязываются к acceptance issue. Пользовательские миры, binaries и JAR
не включаются в Git.
