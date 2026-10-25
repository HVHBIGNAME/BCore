# NOISE 26.2: first-axis delta experiment — 25 октября 2026

**Кандидат отклонён, исходный код восстановлен побайтно.** Проверяемая цель
[#2](https://github.com/HVHBIGNAME/BCore/issues/2) — обойти Java на 1/2/4 workers —
остаётся открытой.

## Изменение и проверки

Профиль выделял `CellInterpolator::sample`. Кандидат вычислял четыре разности
первой оси на cell miss и сохранял их вместе с четырьмя bases в прежнем eight-f64
payload. Восемь corner reads, publication valid bit, cache footprint и порядок
material Y/X/Z либо density X/Y/Z сохранялись. Generic interpolation, FMA,
reassociation, RNG, worker budgets и timer boundaries не менялись.

Правильный `density::chunk::tests` запуск выполнил **10 tests**, включая IEEE/
signed-zero/fraction edges, exhaustive root bits/cold-reverse order, unwind и ore
reader-demand order. **9 native numeric/ore/RNG tests** также прошли. Первоначальный
`density::chunk_tests` filter выбрал 0 tests; это сохранённая ошибка harness,
которая не считается bit proof.

## Измерения

Schema 2, `noise-fill-wg-v2`, seed `846692123413862008`, восемь прежних chunks,
default release flags, warm10/measured10 и 3 fresh processes/configuration.
Каждый полный run включает Java и BCore, а frozen confirmations — только BCore.

| Workers | Before BCore / Java, chunks/s | Candidate BCore / Java, chunks/s | Frozen ratio 1 | Frozen ratio 2 |
|---|---:|---:|---:|---:|
| 1 | 43.24 / 47.59 | 42.88 / 47.61 | 1.012324× | 1.013258× |
| 2 | 77.32 / 119.60 | 79.01 / 83.84 | 1.037165× | 1.013922× |
| 4 | 138.66 / 233.97 | 146.94 / 192.66 | **0.999032×** | **0.992884×** |

На 4 workers оба paired ratio ниже 1; условие повторяемого gain на всех budgets
не выполнено. Небольшие сигналы на 1/2 не превращаются в принятый общий gain.
Кандидат не обошёл same-run Java ни на одном budget.

Все **72 process records**, **720 warmup / 720 measured batches** совпали по blocks,
ordered fluid marks и обеим WG maps. Per-process medians, source/archive/binary
hashes и raw-result identities опубликованы в
[метрике](metrics/noise-axis-delta-rejected-26_2-2026-10-25.json). Полные durations,
requests, logs и fingerprints сохранены в соответствующих raw results под `target/`.

Host не изолирован; fixed-warmup convergence и причина заметной Java variation
не установлены. Это отдельные measurements, а не замена October 23 baseline и
не оценка полного сервера/TPS/gameplay. Обе изменённые source files восстановлены
по SHA-256 frozen before snapshot; в production delta-оптимизация не оставлена.

Следующая граница #2: новая профильная гипотеза с тем же exact-output contract,
worker budgets и повторными frozen confirmations. Historical rejected cache,
cell-miss, constant-operand и Perlin variants остаются отклонёнными.
