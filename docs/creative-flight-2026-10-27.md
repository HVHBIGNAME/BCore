# Чанки перед летящим игроком — 27 октября 2026

Исправление отвечает на ручную жалобу: при полёте в креативе игрок догонял край
загруженной местности, хотя сервер показывал 20 TPS. Проверяемая граница —
**BCore 26.2 / protocol 776 / data 4903**, движущийся TCP-клиент и реально полученные
terrain packets. [Метрика](metrics/creative-flight-validation-2026-10-27.json) и
[acceptance #7](https://github.com/HVHBIGNAME/BCore/issues/7) содержат исходные identities.

## Результат

- **945 passed / 0 failed / 4 ignored**, 72 suites; 233 focused protocol/TCP tests.
  Во время полного прогона сохранены все 871 source inputs и **5070** файлов
  пользовательских миров/операторов, включая membership.
- В трёх 30-секундных полётах по новой местности минимальный измеренный запас
  впереди составил **72 / 72 / 80 блоков**, медиана — **88 / 88 / 88**.
  До исправления минимум был **0 / 8 / 8**, медиана — **16 / 24 / 24**.
- Всего в восьми сессиях выбранной сборки — **2522 samples, 0 пропусков текущего
  чанка**, все ближайшие **3×3 чанка загружены** в каждом sample. Это три fresh,
  три saved, один разворот и один более длинный полёт.
- **60 секунд** полёта: 978 movement packets, 559 samples, 678 terrain packets;
  запас **min 80 / median 96 / max 104 блока**. Разворот на 15-й секунде отдельной
  сессии также прошёл без пропусков, с минимумом 80 блоков впереди.
- Во всех **70 reporting windows** новых flight sessions записано **20,0 TPS**.
  Отдельная нагрузка: **490 настоящих установок/ломаний**, 170 чанков, все изменения
  подтверждены в правильном порядке; после clean shutdown в BCC прочитан AIR.

## Что изменилось

1. У каждой connection одна обновляемая очередь спроса: максимум **64 позиции** и
   один queued/running continuation. Он выбирает следующий target из актуального
   списка; новые движения заменяют ещё не принятые запросы. Drop connection отменяет
   её оставшийся спрос, сохраняя запросы других игроков и shared reservations.
2. Первые по приоритету — ближайшие **3×3**. Затем идёт коридор по **фактическому
   движению**: до шести чанков вперёд, half-width 1,5 чанка. Остальной view сохраняет
   прежний nearest-first порядок. При развороте приоритет меняется; teleport очищает
   старое направление и ждёт matching confirmation. Запросы остаются внутри view.
3. Sender выбирает готовые чанки среди всего missing view, поэтому один неготовый
   ближний чанк не скрывает готовых соседей. Число отправок по-прежнему ограничено
   client credit и одним outstanding wire batch.
4. Обновление спроса и retirement не зависят от наличия send credit. Socket timeout
   привязан к следующему 50 ms deadline, чтобы чтение не добавляло ещё один полный
   интервал задержки.

Приоритет относится к admission целых, ещё не начатых targets. Уже принятый target
проходит прежний порядок source stages. **396 файлов worldgen и registry совпали
побайтно с baseline-архивом**; численная генерация, RNG и biome-tie history в этом
checkpoint не переписаны.

## Метод и полные сравнения

Windows 11, Ryzen 7 5700X, 8 cores / 16 logical, unisolated host. Все тяжёлые
процессы идут последовательно через ROOT queue, `RAYON_NUM_THREADS=2`, обычный
release profile. Seed `846692123413862008`. После первого terrain packet —
**15 секунд warmup**, затем движение **21,78 блока/с** по диагонали
`[-0.9284766908852594, -0.3713906763541038]`. Клиент просит 16 chunks/tick.

Fresh input — metadata и девять saved spawn columns; остальное генерируется.
Saved input — metadata и все 2111 имевшихся BCC columns. Каждый процесс получает
новую копию, original worlds и `ops.json` проверяются по hashes и membership.
Saved mode не утверждает, что весь будущий маршрут заранее сгенерирован.

`lead` — расстояние до первого ещё не полученного чанка вдоль луча движения:
шаг измерения **8 блоков**, верхний предел **136**. Это sampled delivery margin,
не дальность GUI-отрисовки. Coverage снимается nominal каждые 100 ms, ping — 250 ms;
реальные числа samples/movement packets сохранены. Время измеряет `perf_counter`.

### Новая местность, 30 секунд

| Run | Terrain packets, включая warmup | Lead min / median / p95 / max, блоки | Current missing | Nearby 3×3 min | Ping p95, ms |
|---|---:|---:|---:|---:|---:|
| before01 | 302 | 0 / 16 / 72 / 104 | 10 / 280 | 2 | 1,148 |
| before02 | 347 | 8 / 24 / 80 / 104 | 0 / 280 | 6 | 1,101 |
| before03 | 354 | 8 / 24 / 80 / 104 | 0 / 280 | 6 | 1,061 |
| ahead01 | 368 | 72 / 88 / 96 / 104 | 0 / 281 | 9 | 1,782 |
| ahead02 | 359 | 72 / 88 / 104 / 104 | 0 / 280 | 9 | 1,684 |
| ahead03 | 371 | 80 / 88 / 104 / 104 | 0 / 280 | 9 | 1,769 |

После первичных запусков выполнен alternating frozen порядок
**before02 → ahead02 → ahead03 → before03**. В поздних baseline-повторах текущий
чанк успевал загрузиться, но запас падал до 8 блоков и ближайшие 3×3 имели пропуски.
Это важная вариация исходного дефекта, а не повод заменить её первым худшим run.

### Сохранённая местность, 30 секунд

| Run | Terrain packets | Lead min / median / max, блоки | Current missing | Nearby 3×3 min | Ping p95, ms |
|---|---:|---:|---:|---:|---:|
| before01 | 1363 | 80 / 128 / 136 | 0 / 280 | 9 | 1,516 |
| before02 | 1393 | 88 / 136 / 136 | 0 / 280 | 9 | 1,407 |
| before03 | 1375 | 80 / 136 / 136 | 0 / 280 | 9 | 1,553 |
| ahead01 | 1646 | 96 / 136 / 136 | 0 / 281 | 9 | 2,017 |
| ahead02 | 1651 | 96 / 136 / 136 | 0 / 281 | 9 | 1,998 |
| ahead03 | 1638 | 96 / 136 / 136 | 0 / 280 | 9 | 2,277 |

Та же alternating последовательность. На common **initially saved** columns
совпали hashes полных terrain payloads: **1144 + 1133 = 2277 пар / 0 различий**.
Это число парных наблюдений; fresh generation при разных request histories
глобально эквивалентной не объявляется.

### Игровая регрессия

30-секундная production-сессия чередовала place/break nominal каждые 50 ms при
активной генерации и client budget 1 chunk/tick. **490 действий = 245 пар**,
ordered updates совпали, все ack получены, final saved state — AIR.
Ack **median 123,329 / p95 185,551 / max 552,359 ms**; все шесть TPS windows — 20,0.
Максимумы gameplay work: 23,99 / 0,97 / 1,97 / 1,10 / 1,07 / 1,28 ms.
Это regression check сохранённой responsiveness/durability, не новый Java benchmark.

## Диагностика и ограничения

Одна лишь refreshable queue дала fresh `min lead 0`, 22/280 missing-current samples
и потому не стала окончательным решением. Её результаты и executable сохранены
как промежуточная диагностика. Первая проверка также поймала generation до teleport
confirmation; после исправления прошли существующий pillar rejection test и весь
233-test focus. Новые шесть тестов проверяют cancellation, ready selection, clock
deadline, движение/разворот, предел view и отказ менять направление по NaN.

Owned-child CPU profile собрал **10065 leaf samples**: 5702 разрешены в BCore,
4363 относятся к неразрешённым system/runtime leaves. Среди наблюдений:
`FeatureRegion::chunk_at_status` 590, `ImprovedNoise::noise` 576, density evaluation
483, biome zoom 385, interpolation 320. Сохранены 34502 activity-filter exclusions
и семь `SuspendThread:5` failures. Это AMD64 IP sampling с PDB, 3 ms loop delay и
`CARGO_PROFILE_RELEASE_DEBUG=1`; sampling меняет исполнение, его durations исключены
из performance comparisons. Inclusive stacks и точные стоимости функций не получены.

Приоритет движения улучшает запас вдоль маршрута; **весь view radius 32 не становится
равномерно готовым**. Ping p95 немного вырос: все distributions, включая максимум
5,161 ms на saved run, сохранены. Этот результат не доказывает ускорение NOISE,
превосходство над Java или плавность GUI-клиента. Full lifecycle, broader world
parity, physics, online auth и остальные фазы остаются в [ТЗ](roadmap.md) и
[#2](https://github.com/HVHBIGNAME/BCore/issues/2).

## Проверки и воспроизведение

Full ROOT gate: format, release workspace build/test, standalone replay build,
Python suites **29 / 3 / 4 / 8 / 21 / 43 / 116 tests**, семь SVG. Все команды,
log hashes, исходные distributions, inputs и snapshots привязаны к метрике.
Native fixtures и пять helper sources остаются byte-exact. `aislop` executable
на хосте отсутствует; full scan/score не получен.

Функциональные регрессии из checkout:

```text
python -B scripts/run_generation_check.py flight-regressions -- cargo test -p bcore-protocol --release --locked -j 1 --lib --test block_actions --test pillar_actions --test join_flow --test chat_flow -- --test-threads=2
```

Локальный production probe находится в
`target/flight-confirmation-20261027-01/probe_creative_flight.py` вместе с frozen
helpers. Он принимает `--executable`, новый `--output`, `--mode fresh|saved`,
`--seconds` и `--turn-at`. Для повтора нужны те же hash-identified world inputs;
raw worlds, binaries и drivers не включены в Git.

| Артефакт | SHA-256 |
|---|---|
| Baseline executable | `710a540f48bc2cb155a24ec61d09c140ab6502e93f770582b618f1836b1f48c0` |
| Measured flight executable | `61752140beea98ef3986bb0c25330350574e366bd4abaee5c9d99d48397f79d1` |
| Source archive, 871 inputs | `9f460190ac07d0599b07735852f8452a06420616d17612c31c99ffe04f10ec32` |
| Full ROOT gate | `1fbb846ea8d40a0a6b86c06dd0a28237d8037d69efe408a326446a55c0ee2be7` |
| Frozen confirmation | `9efd3d654c7a00a25728648b41f6e68da4fa5b35b9785da305986eb61a999763` |

Публикационные документы добавляются после measured snapshot. Окончательный index
проходит independent exported-byte build/tests, затем exact-commit CI/Pages и live
readback; эти receipts и запуск существующего пользовательского мира привязываются
к acceptance issue. Исторические checkpoints 886/903/921/927/939 сохраняются.
