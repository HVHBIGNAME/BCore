# Референсные проекты

Сравнение проектов-ориентиров для BCore (нативный Rust-сервер, vanilla-паритет, 26.2).

| Проект | Язык | Лицензия | Протокол | Worldgen-паритет | Плагины | Что взять |
|---|---|---|---|---|---|---|
| **SteelMC** | Rust | AGPL-3.0 | 26.2 (776) | почти готов (парити-сьют, 7500 чанков) | нет | архитектура мультикрейта, worldgen-методика, многопоточность |
| **Pumpkin** | Rust | MIT | Java + Bedrock | частично | свой API-фундамент | протокол (Java+Bedrock), MIT-совместимость |
| **Valence** | Rust | MIT | последний stable | библиотека, частично | фреймворк | NBT, chunk, клиентская сторона |
| **Cuberite** | C++ | Apache-2.0 | Java + Bedrock (старее) | свой генератор | Lua | подход к плагинам на скриптах |
| **Glowstone** | Java | MIT | ~1.21 | через Bukkit | Bukkit API | открытая реализация Bukkit API (для транслятора) |

## Выводы

- **SteelMC** — самый близкий ориентир по цели (26.2, Rust, vanilla-паритет). Но лицензия AGPL-3.0: BCore пишется независимо, как отдельная реализация, а не форк.
- **Pumpkin** — MIT, даёт свободу заимствования идей протокола (Java+Bedrock) и плагинного фундамента.
- **Valence** — MIT-фреймворк; полезен как библиотека-справочник по NBT/chunk/пакетам, но BCore строит собственную реализацию ради полного контроля.
- **Glowstone** — открытая Java-реализация Bukkit API: главный референс для JNI-заглушек в `bcore-plugin-java`.

## Источники

- https://github.com/Steel-Foundation/SteelMC
- https://github.com/Pumpkin-MC/Pumpkin
- https://github.com/valence-rs/valence
- https://github.com/cuberite/cuberite
- https://github.com/GlowstoneMC/Glowstone
