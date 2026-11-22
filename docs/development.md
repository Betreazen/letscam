# Разработка LetsCam

Как устроен мод, как его собрать и проверить. Полная спецификация и журнал проверок — [design/01-spec.md](design/01-spec.md).

## Устройство

С версии 0.2.0 LetsCam — нативный мод на [Mod API Horizon](https://github.com/horizon-rust/horizon/tree/main/docs/modding/src). Лаунчер собирает его против готовой игры из SDK той сборки Horizon, к которой мод привязан. Саму игру он не пересобирает. Версии 0.1.x собирались старым бандлом `horizon-mixin.toml` и работают только на Horizon до слияния Mod API (лаунчер до 0.2.69).

**`mod/letscam/` — плоский проект мода.**

| Файл | Что это |
|---|---|
| `horizon-mod.toml` | манифест: id, версия, `side = "client"`, привязка `[native]` к сборке Horizon и инструменту horizon-mixins, 7 модулей и 9 миксинов |
| `Cargo.toml`, `Cargo.lock` | крейт `letscam-engine`, он и есть крейт мода; зависит только от `horizon-mod-api` и пакетов с crates.io |
| `src/` | логика: запись, камера-режиссёр, кодировщик, листы кадров |
| `src/mixins/` | хелперы `[[native.mixin]]`: по одной функции, вызывают модули |
| `src/modules/` | модули `[[native.module]]`: компилируются внутри `crate::game` и видят закрытые поля игры |
| `src/bridge.rs` | передача кадра от считывания скриншота к модулям |

Модули игры:

| Файл | Что делает |
|---|---|
| `glue.rs` | запись, рендер, клавиши, команды, HUD |
| `screen.rs` | окно записей (F10) |
| `edit.rs` | редактор ключевых кадров |
| `world.rs` | время суток и реестры для реплея |
| `shot.rs` | фокус и поза камеры рендера |
| `labels.rs` | фазы атак SoulsCraft для листов |
| `diag.rs` | диагностика синхронизации модели игрока в логе |

Миксины и точки игры, в которые они встают:

| Миксин | Функция игры | Зачем |
|---|---|---|
| `save_screenshot` | `game::chat::Game::save_screenshot` | кадр с маркером `letscam-capture:` уходит в `bridge`, а не в файл |
| `advance_replay` | `game::replay::Game::advance_studio_replay` | реплей сдвигается ровно на кадр ролика |
| `frame` | `native_hooks::render_frame_pre` | каждый кадр |
| `key` | `native_hooks::input_key` | клавиши записи и окна |
| `command` | `native_hooks::base::native_command` | `/letscam` |
| `hud` | `native_hooks::render_gui_post` | значок REC |
| `screen_render_post` | `game::screen_backgrounds::draw_screen_with_post_hook` | окно записей рисуется здесь: `native_screen_render_post` зовётся, только если мод подписан на хук через трейт |
| `screen_mouse_button_pressed_pre` | `native_hooks::base` | клики в окне записей |
| `payload_recorded` | OVERWRITE `game::remote::raw_ingress::Remote::receive_credited_backlog` | сообщения `soulscraft:state` пишутся в реплей, пока идёт запись: рекордер студии их не видит |
| `replay_payload` | `game::remote::apply_packet` | при рендере записанные `soulscraft:state` идут в подписи листов |
| `attachment_sync_recorded` | `game::attachment_sync::remote_attachment_sync` | пакеты вложений пишутся в реплей сами, рекордер студии их теряет |

Клавиши F9 и F10 регистрирует сам крейт (`Hooks::register` в `lib.rs`), их можно переназначить в «Управлении».

Серверной части нет: мод только клиентский.

## Сборка

Нужны лаунчер Horizon с Mod API и установленная сборка, к которой привязан мод (`[native] horizon` в `horizon-mod.toml`, сейчас Nightly 2026-11-21, `8f83e5c`).

```powershell
horizon-launcher install nightly-2026-11-21-8f83e5c
horizon-launcher prepare --version nightly-2026-11-21-8f83e5c
horizon-launcher native-build mod\letscam --version nightly-2026-11-21-8f83e5c --trust-native-code
```

Первая сборка скачивает части SDK (около 300 МБ) и Rust лаунчера. Дальше на этом ПК (Ryzen 9 5950X):
- правка модуля или хелпера — ~30 с;
- правка крейта — ~1 мин 50 с: крейт собирается в release;
- полная сборка из пустого кэша, без скачивания, — ~3 мин 20 с.

Полный лог сборки — `native/build.log` в папке лаунчера. Для путей с пробелами лаунчер собирает в ASCII-папке, например `C:\ProgramData\horizon-launcher-<hash>\native\build.log`.

### Ловушки

- `#[mod_api::horizon_mod] impl Hooks` должен стоять в `lib.rs`, не в модуле.
- `capabilities = ["registries"]` обязательно в каждом манифесте, даже если мод ничего не регистрирует.
- Хелпер с `cancellable = true` возвращает `core::ops::ControlFlow<…>`, и тип внутри совпадает с типом функции игры буква в букву.
- Модули можно ставить только в файлы `crates/horizon-client/src/` (крейт игры). В рендер-крейт нельзя, поэтому `bridge.rs` живёт в крейте мода.
- **`Cargo.lock`.** Версии пакетов, общих с игрой, должны совпадать с её `Cargo.lock`. `horizon-launcher native-lock` берёт самые свежие версии, и сборка их отвергает («game or pinned package … moved»). Поэтому `Cargo.lock` мода сделан из `Cargo.lock` игры той же сборки (`horizon-source.tar.gz` в релизе Nightly): скопировать его в `mod/letscam/` и выполнить `cargo metadata`, чтобы cargo убрал лишнее.
- Цвет текста в HUD — ARGB: без байта альфы текст не рисуется.

### Переход на новую сборку Horizon

Миксины привязаны к одной сборке. Чтобы перейти на новую:
1. Поменять `horizon` в `[native]` (или `cargo horizon-mod update --horizon <build>`).
2. Заново сделать `Cargo.lock` из `Cargo.lock` игры.
3. Собрать: переименованные цели миксинов и поля `Game` дадут ошибки с именем мода.
4. Перечитать функции игры, на которые стоят миксины: изменения внутри них сборка не заметит.

## Тесты крейта

Один раз после клонирования — `cargo horizon-mod check` в `mod/letscam`: он скачивает SDK в `.horizon/sdk/`, без него крейт не находит `horizon-mod-api`.

```bash
cd mod/letscam
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run          # или cargo test
```

Среди тестов:
- реальные записи дома и капсулы RustCraft, прогоняемые кадр за кадром (`director/tests_scenes.rs`);
- сборка роликов и листов кадров настоящим ffmpeg.

Тестам с ffmpeg нужен `ffmpeg` в `PATH` или `C:\ffmpeg\bin`.

## Проверка в игре

`tools/bench-test.ps1` поднимает свой одноразовый сервер той же Nightly: без модов, офлайн, только на 127.0.0.1. Офлайн-бот `lc_a` играет сборкой с LetsCam. Сценарий:
- запись ~40 с: бот подходит к домику, проходит в дверь, открывает инвентарь, выходит;
- редактор ключей сохраняет 3 ключа;
- рендер `auto+orbit+side+top+keys sheet`;
- проверка ролика ffprobe.

Скрипт рассчитан на изолированный лаунчер в `C:\Claude code projects\horizon-sdk-dev` (`hl.ps1` задаёт ему свои папки данных и выключает самообновление), путь меняется параметром `-SdkDev`. Нужен pwsh 7. Пока `C:\ai-bridge\heavy.lock` занят чужим замером производительности, скрипт ждёт.

```powershell
pwsh -File tools\bench-test.ps1
```

Сценарии:
- никогда не пишут настоящий микрофон (`record_mic = false`);
- запускают бота без звука;
- кладут ролики в `F:\letscam\modapi-bench`.

После рендера в логе клиента остаются строки:
- `letscam: render stats` — тайминги кодировщика: `submit` — время LetsCam на кадр в потоке игры, в скобках его части (картинка в картинке и запуск ffmpeg); `queue wait` — ожидание, пока ffmpeg догонит; `pipe write` — запись в ffmpeg в отдельном потоке;
- `letscam: render loop` — кадры игры на кадр ролика, повторы захвата, время кадра и дрейф реплея.

Дрейф считается только по кадрам до конца реплея. Реплей кончается своим последним пакетом, обычно на десятки миллисекунд раньше сессии записи. Последние кадры ролика смотрят за этот конец, и раньше их ошибочно считали дрейфом.

`horizon-ctl` нажимает только F1–F6 и F11, а не F9 и F10. Поэтому стенд переназначает клавиши LetsCam так, как это сохраняет «Управление» (строки `key.letscam.toggle:` и `key.letscam.menu:` в `horizon-mods.txt` рядом с `options.txt`), на F6 и F4. После рендера бот открывает окно по F4 и делает короткую запись по F6. Основную запись он останавливает командой `/letscam stop`.

`tools/capsule-scene.ps1`, `tools/sync-check.ps1` и `tools/lib.ps1` — сценарии старого общего стенда `horizon-joint-dev` (Horizon 7e4e421, версии 0.1.x). Капсуле нужен RustCraft на сервере. Она вернётся, когда RustCraft переедет на Mod API.

## Известные ошибки и пробелы

| Что | Что делает LetsCam |
|---|---|
| рекордер студии теряет пакеты модов ([#16039](https://github.com/horizon-rust/horizon/issues/16039)) | пишет пакеты вложений (`horizon:attachment_sync`) в реплей сам |
| отдельные реплеи теряют время суток ([#16040](https://github.com/horizon-rust/horizon/issues/16040)) | сохраняет его в `world_time.json` |
| очередь рекордера на 2048 пакетов переполняется, если диск встаёт, и запись обрывается ([#16041](https://github.com/horizon-rust/horizon/issues/16041)) | сразу пишет об этом в чат |
| модель своего игрока забегает вперёд в клетке открытой двери ([#16527](https://github.com/horizon-rust/horizon/issues/16527)) | открыто |
| сообщения модов идут мимо рекордера студии | `payload_recorded` пишет `soulscraft:state` в реплей сам; в игре подписи не проверены |
| два нативных мода с общими crates.io-крейтами (serde, log) не линкуются в одну игру ([#19616](https://github.com/horizon-rust/horizon/issues/19616)) | LetsCam и SoulsCraft нельзя собрать вместе; `tools/labels-check.ps1` (бой ревенанта на Linux-сервере SoulsCraft, подписи на листах) ждёт исправления |
| миксин на `receive_client_payload` не собирается: генератор патча экспортирует `const _` модуля ([#19505](https://github.com/horizon-rust/horizon/issues/19505)) | OVERWRITE `receive_credited_backlog` повторяет её строки на 8f83e5c; при переходе на новую сборку их надо сверить |
| кадр в рендере на Nightly 8f83e5c ~19–20 мс против ~13 мс на 7e4e421 | причина не найдена |
