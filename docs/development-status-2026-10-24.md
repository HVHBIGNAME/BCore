# Статус разработки — обновлено 25 октября 2026

Цель проекта: все механики **Minecraft Java 26.2 / protocol 776 / data version
4903**, с проверкой поведения против закреплённого оригинального сервера.
Полный список фаз находится в [ТЗ и дорожной карте](roadmap.md), текущие состояния
подсистем — на [странице задач](https://hvhbigname.github.io/BCore/).

## Выполнено и проверено

- Переход общих registry IDs, protocol admission и BCC v6 на 26.2; исторические
  эталоны 26.1 остаются явно историческими.
- Migration checkpoint: **886 passed / 0 failed / 4 ignored**.
- Canonical BCC admission и shared saved lighting: новый workspace checkpoint
  **903 passed / 0 failed / 4 ignored**, 70 suites; 13 admission integration tests.
  Исправлены потеря pending futures, отсутствующий shared light import и ложная
  FEATURES history flag. [Отчёт admission](saved-chunk-admission-2026-10-25.md).
- Нативные codecs декодировали **1179 пакетов**, включая **127 terrain packets**,
  в трёх сессиях BCore; сохранённое изменение блока наблюдалось после restart.
- Новый NOISE baseline: совпали blocks, ordered marks и обе WG maps во всех
  **18 процессах**. BCore/Java при 1/2/4 workers: **47,79/49,77**,
  **83,98/74,03**, **156,81/215,35** чанка/с.
- Свежий owned-child CPU-профиль: **4002 leaf samples**; подробные ограничения
  и исходные hashes сохранены в [отчёте](performance-profile-26.2-2026-10-24.md).
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
| Ломка/установка блоков | Planned; authoritative mutation, ответы клиенту/viewers, dirty save и restart/reload |
| Физика и инвентарь | Planned; AABB/collision/gravity, hotbar/slots, menu ownership и click transactions |
| Игрок, mobs, redstone и жидкости | Открыты соответствующие фазы дорожной карты |
| Скорость относительно Java | Цель открыта: Java быстрее на 1 и 4 workers; кандидат принимается только после exact-output gate и повторных парных замеров |
| GUI-клиент и online auth | Не проверены/не реализованы полностью |

## Проверки публикации

CI дополнен сборкой standalone replay и переносимыми Python evidence/publication
проверками. Для новых коммитов требуется пройти фактический CI и проверить Pages.
Исторические hashes и счётчики не заменяются результатами новой сборки.

Исполняемый `aislop` на локальном хосте не найден: полный scan не выполнен.
Отсутствие вывода hook не считается успешным scan.
