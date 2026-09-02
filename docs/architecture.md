# BCore — архитектура

Нативный сервер Minecraft Java Edition на Rust. Цель — **26.2** (протокол **776**), vanilla-паритет механик, нативная плагинная система и транслятор Bukkit/Spigot/Paper-плагинов. Вдохновлён SteelMC, но пишется независимо (SteelMC — AGPL-3.0, не форк).

## Карта крейтов (bounded contexts)

| Крейт | Назначение | Статус |
|---|---|---|
| `bcore` | бинарь, CLI (`--host`/`--port`), запуск TCP-листенера | работает (статус-пинг) |
| `bcore-core` | общие типы: версии, VarInt/VarLong, позиции, `Identifier`, bootstrap-реестр | работает |
| `bcore-protocol` | протокол 776: handshake/status/login, фрейминг, TCP-сервер | работает (статус+пинг) |
| `bcore-plugin` | нативная плагинная система: `Plugin`, `PluginManager`, события, динамическая загрузка | работает (in-process), dyn-загрузка — прототип |
| `bcore-plugin-java` | JVM-мост для Bukkit/Spigot/Paper плагинов (виртуализация) | прототип |
| `bcore-worldgen` | детерминированная seed-генерация мира | прототип (не vanilla-паритет) |
| `bcore-registry` | data-driven реестр блоков/предметов (registry sync) | заглушка |

## Поток данных

```
клиент → TCP → bcore-protocol (handshake → status | login)
                    ↓
              bcore-core (общие типы)
                    ↓ (в перспективе)
              мир/генерация (bcore-worldgen) + реестры (bcore-registry)
                    ↓
              плагины (bcore-plugin / bcore-plugin-java)
```

## Модель потоков

- **Текущая**: thread-per-connection для протокола (`std::net` + `std::thread`) — достаточно для статус-пинга и ранней разработки.
- **Целевая** (по образцу SteelMC): gameplay-тик синхронный (vanilla-детерминизм), а генерация чанков, освещение, обработка пакетов и отправка чанков — вне главного тика, на нескольких ядрах. Сетевой слой планируется перевести на `tokio`.

## Плагинная система

- **Нативный путь** (`bcore-plugin`): трейт `Plugin`, `PluginManager`, события, динамическая загрузка `.dll`/`.so` через C ABI (`bcore_plugin_create` / `bcore_plugin_metadata`). Рекомендуемый будущий бэкенд — **WASM** (wasmtime): песочница, кроссплатформенность, единая цель для транслятора.
- **Совместимость** (`bcore-plugin-java`): встроенная JVM грузит оригинальные Bukkit/Spigot/Paper jar, переопределяя их API как JNI-заглушки → нативное ядро (см. `docs/adr/0001-plugin-translation-strategy.md`).

## Решения

- Rust **stable** (не nightly), edition 2021 — намеренно проще для старта; nightly добавим под конкретные фичи.
- Константы версии централизованы в `bcore-core/src/version.rs` (`PROTOCOL_VERSION = 776`, `MC_VERSION = "26.2"`).
- Data version (`DATA_VERSION`) пока приблизительный — уточнить перед registry-sync/login-геймплеем.
