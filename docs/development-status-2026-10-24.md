# Статус разработки — обновлено 27 октября 2026

Цель проекта: все механики **Minecraft Java 26.2 / protocol 776 / data version
4903**, с проверкой поведения против закреплённого оригинального сервера.
Полный список фаз находится в [ТЗ и дорожной карте](roadmap.md), текущие состояния
подсистем — на [странице задач](https://hvhbigname.github.io/BCore/).

## Выполнено и проверено

- Creative flight: **945 passed / 0 failed / 4 ignored**, 72 suites, 871 source inputs
  и **5070** world/operator files сохранены. Refreshable demand, ready-chunk selection
  и приоритет движения: восемь frozen TCP-сессий, **2522 samples / 0 missing-current**,
  ближайшие 3×3 загружены всегда; fresh minimum **0/8/8 → 72/72/80 блоков**.
  Минутный полёт держит минимум 80 блоков впереди. Все новые flight windows — 20 TPS;
  490 actual edits с durability/readback также прошли. Полный view/GUI и Java остаются
  отдельными проверками. [Отчёт полёта](creative-flight-2026-10-27.md).
- Runtime responsiveness: **939 passed / 0 failed / 4 ignored**, 72 suites,
  868 unchanged inputs и 4592 unchanged world/operator files. Nonblocking action queue,
  generation handoff, incremental framing, client-paced chunks и revisioned async
  saves с durability ack. 32-break median **1881 → 244 ms**; production correction
  p95 **1718/1781 → 141/187 ms**, 20 TPS во всех новых окнах. Проверены 488 реальных
  place/break actions, 171 чанк и persisted AIR. Median correction выросла 93 → 125 ms,
  доставлено меньше чанков; Java comparison остаётся отдельной целью.
  [Отчёт runtime](runtime-latency-2026-10-27.md).
- Переход общих registry IDs, protocol admission и BCC v6 на 26.2; исторические
  эталоны 26.1 остаются явно историческими.
- Migration checkpoint: **886 passed / 0 failed / 4 ignored**.
- Canonical BCC admission и shared saved lighting: новый workspace checkpoint
  **903 passed / 0 failed / 4 ignored**, 70 suites; 13 admission integration tests.
  Исправлены потеря pending futures, отсутствующий shared light import и ложная
  FEATURES history flag. [Отчёт admission](saved-chunk-admission-2026-10-25.md).
- Authoritative creative break/place через реальные packets: **921 passed / 0
  failed / 4 ignored**, 71 suites и 517 unchanged source inputs. **37 focused tests**,
  включая **6 TCP gameplay scenarios**: selected hotbar, second viewer, canonical
  edit, dirty save, restart, rejection и first-delivery race. Native wire authority:
  **25 cases / 197 stateless bindings**. [Отчёт gameplay](creative-block-gameplay-2026-10-25.md).
- Axis-only creative placement/break: **927 passed / 0 failed / 4 ignored**,
  **72 suites / 526 unchanged source inputs**. **43 focused tests**, включая
  **3 новых TCP scenarios**; native authority — **336 repeat-matching cases / 56
  items / 168 states**. Вместе поддержаны **253 full-cube items / 365 states**.
  Все шесть faces, pillar supports, X/Y/Z break, viewers, chunk border и restart
  проверены. [Отчёт pillar](creative-pillar-gameplay-2026-10-26.md).
- Нативные codecs декодировали **1179 пакетов**, включая **127 terrain packets**,
  в трёх сессиях BCore; сохранённое изменение блока наблюдалось после restart.
- Новый NOISE baseline: совпали blocks, ordered marks и обе WG maps во всех
  **18 процессах**. BCore/Java при 1/2/4 workers: **47,79/49,77**,
  **83,98/74,03**, **156,81/215,35** чанка/с.
- Свежий owned-child CPU-профиль: **4002 leaf samples**; подробные ограничения
  и исходные hashes сохранены в [отчёте](performance-profile-26.2-2026-10-24.md).
- Проверен first-axis delta candidate: **19 bit/order/native tests**, **72 matching
  processes** и два alternating frozen confirmations. На 4 workers ratio
  **0,999032× / 0,992884×**: кандидат отклонён, production sources восстановлены
  побайтно; Java goal открыт. [Отчёт](performance-experiments-26.2-2026-10-25.md).
- Два seeded-cache experiments прошли по **21 bit/order/native tests** и
  **126 matching processes / 1260 warmup / 1260 measured batches**. Hit-return
  регрессирует на 4 workers в обеих frozen confirmations; dense index даёт смешанные
  1/2-worker results. Оба отклонены, все **510 baseline inputs** восстановлены
  побайтно. [Отчёт](performance-experiments-26.2-2026-10-26.md).
- Очистка: удалены **18,76 GiB** логических файлов — debug incremental cache,
  **33** побайтно проверенные распакованные копии архивов и **27** временных
  каталогов завершённых тестов. Свободное место: **115,64 → 132,85 GiB**.
  Проверены **2958** файлов мира и операторов: bytes не изменились. `source.zip`,
  frozen executables, raw captures и журналы сохранены.
  [Метрика очистки](metrics/storage-cleanup-2026-10-24.json).
- Новые byte-pinned native JSON fixtures исключены из Git EOL-нормализации. Существующая LF-политика
  для кода и опубликованных JSON-отчётов сохранена; исходные native hashes не менялись.
- В standalone replay snapshot включён shared registry crate; path dependencies
  проверяются перед сборкой и при reuse. **43** Python replay-теста прошли, включая
  отказ при отсутствующей транзитивной зависимости и выходе за архив.

## Открытые этапы ТЗ

| Этап | Статус и ближайшая проверяемая граница |
|---|---|
| Полный Overworld и ticket-driven FULL | Partial; нужны сохранение/восстановление canonical holders и полные межчанковые истории |
| Saved-column admission | Выполнен data-admission slice: canonical Level/Imposter data, futures/cache, saved neighbours, stored light и restart проверены; generation history/registration остаются Partial |
| Полёт и выдача чанков | Проверены refreshable demand, приоритет движения и 30/60-second TCP trajectories; 2522 samples без пропусков ближайших 3×3. Равномерная готовность всего view и GUI rendering требуют дальнейших проверок |
| Ломка/установка блоков | Creative stateless + axis-only full-cube slices: 253 items, grass/podzol/mycelium supports и break, bounded nonblocking requests, tick-owner mutation, durable ack/correction/viewers, async save/restart; прочие stateful blocks/neighbour callbacks/survival открыты |
| Физика и инвентарь | Standing overlap и creative unpatched hotbar 0..8 для 253 full-cube items проверены; полные poses/collision/gravity, menus/click transactions и persisted playerdata открыты |
| Игрок, mobs, redstone и жидкости | Открыты соответствующие фазы дорожной карты |
| Скорость относительно Java | Цель открыта: два seeded-cache candidates отклонены; runtime before/after BCore уменьшил длинные паузы и прошёл 20 TPS нагрузки. Нужны repeatable all-budget gains и matched Java runtime checks |
| GUI-клиент и online auth | Не проверены/не реализованы полностью |

## Проверки публикации

CI дополнен сборкой standalone replay и переносимыми Python evidence/publication
проверками. Для новых коммитов требуется пройти фактический CI и проверить Pages.
Исторические hashes и счётчики не заменяются результатами новой сборки.

Исполняемый `aislop` на локальном хосте не найден: полный scan не выполнен.
Отсутствие вывода hook не считается успешным scan.
