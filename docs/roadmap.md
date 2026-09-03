# BCore — дорожная карта переноса ванильных механик на натив

> Цель: собственная нативная реализация всех механик Minecraft 26.2 (протокол 776),
> **не** копирование API/кода 1в1, а инженерная реимплементация с parity-тестами против
> ванильного сервера. Философия — как у SteelMC: `reproduce vanilla where vanilla is
> deterministic, fix accidental nondeterminism where it changes nothing`.

## Эталоны (ground truth)

| Источник | Роль | Лицензия | Как использовать |
|---|---|---|---|
| `target/vanilla-26.2-server.jar` | живой ванильный сервер | проприетарная Mojang | parity-тесты, захват пакетов |
| [PaperMC/mache](https://github.com/PaperMC/mache) | декомпилированный сервер (Mojang mappings + Parchment) | LGPL-3.0 + Mojang proprietary | **читать** ванильную логику, портировать *поведение*, не копировать код |
| [SteelMC](https://github.com/Steel-Foundation/SteelMC) | референс архитектуры (worldgen pyramid, parallel lighting, native plugins) | AGPL-3.0 | смотреть подход (не копировать, у нас MIT) |
| Paper/Spigot patches | оптимизации (Moonrise, Alternate Current, hoppers) | GPL/MIT (патчи) | переносить *идеи* оптимизаций в натив |
| [anvil-nbt](https://github.com/driedpampas/anvil-nbt) | NBT/Anvil библиотека | **GPL-3.0** | ⚠️ НЕ использовать — GPL несовместим с MIT |

### Лицензионное решение (важно)
- **anvil-nbt — GPL-3.0.** Включение в MIT-проект делает весь проект GPL. У нас уже есть
  собственный `nbt.rs` (bcore-protocol) — развиваем его, а не тянем GPL-зависимость.
  (Альтернатива: перейти на AGPL как SteelMC — решить позже, сейчас MIT.)
- **mache — LGPL + Mojang proprietary.** Декомпиляция неточна. Используем строго как
  *справочник* для понимания семантики (порядок операций, формулы), реализуем своё.

## Текущее состояние BCore (v0.6.0)

Готово: net+login+join, чат, команды (ванильный набор, селекторы, scoreboard, RU),
генерация мира v1 (6 фаз), сохранение чанков, view distance 20, TPS 20, stdin-консоль,
спектатор, F3+F4, бан-система, энтити-модуль (заглушки), native plugin API + JVM-мост (M1).

Не готово (по приоритету): мир 1в1, физика/коллизии, блоки, инвентарь, голод/эффекты/зелья,
свет, mob AI, блок-энтити (сундуки/печи), редстоун, жидкости, детерминированный RNG.

## Архитектурные принципы (заимствованы у SteelMC, адаптированы)

1. **Игровой тик — синхронный, одиночный поток** (как ваниль). Chunk gen, lighting,
   сжатие/отправка пакетов, сохранение — вне main tick (параллельно).
   Это даёт ванильную детерминированность + масштабируемость. НЕ Folia (экспериментально).
2. **Worldgen — chunk pyramid** (12 стадий: Empty → Structure Starts → … → Full),
   зависимости 23×23 → 7×7 → 5×5 → 3×3 → 1×1. Стадии обрабатываются параллельно по
   порядку. Сейчас у нас однофазная генерация — переписываем на стадийный конвейер.
3. **Детерминированность** — устранить источники ванильного nondeterminism:
   - tied biome lookup (thread-local cache) → chunk-local cache;
   - hash-based collections iteration order → явный порядок;
   - cross-chunk features (деревья/руды) → фиксированный порядок генерации соседей.
4. **RNG** — реализовать `java.util.Random` (48-bit LCG, seed semantics) в Rust 1в1,
   чтобы parity была достижима. Плюс Paper-патчи, фиксящие RNG-manipulation дыры.
5. **Свет** — параллельный движок (Starlight/ScalableLux-подход), chunk-local с
   координацией перекрытий через ownership.
6. **Редстоун** — сначала ванильная логика (с её полезными багами, как SteelMC),
   затем Alternate Current как оптимизирующий слой (отключаемый), не меняющий результат.
7. **Плагины** — native Rust API (первичный) + JVM-мост для **Bukkit API** (стабильный
   слой, не NMS — NMS обфусцирован и меняется каждую версию). Быстрая загрузка 100+
   плагинов — ленивая инициализация + кеш + параллельная загрузка jar.

## Дорожная карта (фазы)

- **Phase 0 — фундамент (сделано):** net/join/чат/команды/генерация v1/сохранение/TPS/консоль.
- **Phase 1 — мир 1в1:** стадийный worldgen (pyramid), block-for-block parity suite
  (7500 чанков, 2500/измерение, как SteelMC). Начать с Overworld noise+surface+features.
- **Phase 2 — игрок-физика:** коллизии (AABB), гравитация, движение, вода/лава задержки,
  ломка/установка блоков, инвентарь (вкладка, стеки, слоты).
- **Phase 3 — игрок-состояние:** голод/насыщение, здоровье/урон/смерть/респаун,
  эффекты (статус-эффекты), зелья, опыт/уровни, атрибуты.
- **Phase 4 — свет:** sky/block light, параллельная пропагация, пересчёт при изменении блоков.
- **Phase 5 — энтити:** спавн (ванильные правила), mob AI (путь, цели, «ванильный интеллект»),
  дроп, item entity, despawn, взаимодействие.
- **Phase 6 — блок-энтити + редстоун:** сундуки/печи/воронки (hopper-оптимизации Paper),
  редстоун (ванильный → Alternate Current).
- **Phase 7 — жидкости + мир-системы:** разлив воды/лавы, тикающие блоки, farm-проверки
  (оптимизации Paper/SparklyPaper: ленивые moisture-проверки).
- **Phase 8 — детерминизм + RNG:** Java Random 1в1, детерминированные hash-коллекции,
  seeded-тесты «одинаковая функция → одинаковый результат».
- **Phase 9 — мета-системы:** gamerule (актуальный набор), advancements, datapack/functions,
  dimensions (nether/end), playerdata (NBT).
- **Phase 10 — платформа плагинов:** native API (стабилизировать) + JVM-мост → Bukkit API,
  параллельная загрузка 100+ jar, event bridging.

## Тестовая стратегия (parity)

Для каждой перенесённой механики — **искусственная среда** (isolated test harness):
1. Скопировать функцию/механику из ванили (через mache/захват).
2. Реализовать в Rust.
3. Запустить обе на одинаковых входах (seed, состояние, вход).
4. Сравнить результат: идентичен (или документированное расхождение/улучшение).
5. Прогнать в CI + parity-suite (для мира — block-for-block по 7500 чанков).

## Оптимизации Paper, которые переносим сразу в натив

- Moonrise: Entity Activation Range 2.0 (не тикать далёких/неактивных мобов).
- Incremental chunk & player saving (не блокировать тик записью на диск).
- Optimize Hoppers (skip events без слушателей, cooldown-when-full, меньше клонирования).
- Alternate Current redstone (directed update graph, -95% block updates).
- Optimize EntityScheduler ticking (не итерировать всех энтити ради пустого планировщика).
- Ленивые moisture-проверки ферм (SparklyPaper) — проверять только при попытке роста.
- RNG-manipulation фиксы (overstacked item filter и т.п.).
