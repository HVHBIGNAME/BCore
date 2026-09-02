# Перенос публичных Java-патчей Paper/Purpur в нативное ядро

Идея: брать проверенные публичные патчи Paper/Purpur (безопасность + оптимизация) и переносить их **логику** в нативный Rust-код BCore. Переносим инвариант и поведение, а не Java-код буквально.

## Категории и пригодность к нативному переносу

### 1. Анти-эксплойты / безопасность — переносятся хорошо
Логика не привязана к JVM: это проверки входных данных и лимиты.
- санитизация входящих пакетов (аномальные клики/анимации/пакеты, tab-complete/chat-инъекции, вредоносные JSON-компоненты);
- лимиты сущностей/блоков на чанк (armor stands, falling blocks, throttle redstone);
- анти-дупы (portal entity exploit, elytra fly infinite durability, AFK-fishing/bartering-лупы);
- анти-глитчи (nether roof, выход за world border, end portal god mode).

### 2. Чанк-система и I/O — переносятся хорошо (в духе SteelMC)
- асинхронная загрузка/сохранение чанков вне главного тика;
- система приоритетов/urgency загрузки чанков (Chunk Priority/Urgency System);
- оптимизации POI, region-file, DataBits, unload при низком TPS.

### 3. Оптимизации тиков/памяти — переносятся, но требуют осторожности
- оптимизация hoppers, voxel shape merging, redstone algorithm;
- Moonrise-оптимизации (dirt/snow spread, getChunkAt для загруженных чанков, pathfinder);
- incremental chunk/player saving, entity bounding-box lookup.

### 4. JVM/Java-специфичные — НЕ переносятся напрямую
- GC-тюнинг, замена Java streams в горячем коде, dataconverter (зависит от Java-объектов/классов).

## Методика

1. Выбрать патч; прочитать vanilla-источник и соответствующий `.patch` в Paper/Purpur.
2. Понять инвариант, который патч защищает/оптимизирует.
3. Переписать логику идиоматичным Rust-кодом в нужном крейте BCore, добавить тест-кейс.
4. Зафиксировать в трекере: патч → версия → статус переноса.

## Источники

- https://github.com/PaperMC/Paper (каталог `patches/`)
- https://github.com/PurpurMC/Purpur
- https://github.com/YouHaveTrouble/minecraft-optimization
