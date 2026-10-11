# Плагины

Плагин — единица поставки любого содержимого Touch Bar. Плагины доверенные: прав, согласий, подписей и установки пакетов нет.

## Состав

Плагин может содержать любую комбинацию частей:

| Часть | Файлы | Назначение |
|---|---|---|
| `logic` | `logic.wasm` | Модуль логики. Один модуль обслуживает несколько наборов (см. [05-logic.md](05-logic.md)) |
| `sets` | `sets/*.toml` | Наборы: раскладка элемента, варианты по ширине, панели |
| `profiles` | `profiles/*.toml` | Профили (см. [08-profiles.md](08-profiles.md)) |
| `categories` | `categories.toml` | Правила категорий приложений |
| `bindings` | `bindings.toml` | Привязки нажатий по умолчанию (см. [09-bindings.md](09-bindings.md)) |

Наборы без логики допустимы: тогда используются только встроенные виджеты и действия, например `key`.

## Манифест

```toml
# plugins/controls/plugin.toml
id = "controls"                 # [a-z0-9-], уникален среди плагинов
version = "1.0.0"
api = "1"                       # версия контракта логики и форматов
name = "System Controls"
description = "Volume, brightness and microphone"
logic = "logic.wasm"            # необязательно
requires = []                   # плагины, на наборы которых ссылаются профили этого плагина

[[sets]]
id = "volume"
file = "sets/volume.toml"

[[sets]]
id = "brightness"
file = "sets/brightness.toml"
```

Ссылки между плагинами пишутся как `plugin/id`: `controls/volume`, `profile-media/media`. Если плагин из `requires` отсутствует или выключен, слоты с его наборами остаются пустыми, а профиль продолжает работать. Это видно в `touchbarctl status` и `explain`.

## Наборы

Формат переносится из `touchbar-set-config` исходного проекта, валидация становится строже (A11: `canvas` требует `canvas_from`). Узлы раскладки:
- контейнеры: `row`, `column`, `stack`, `responsive` (варианты по ширине), `repeat` (по записям состояния);
- элементы: `button`, `text`, `icon`, `image`, `slider`, `meter`, `progress`, `scrubber`, `canvas`, `shader-effect`;
- промежутки: `fixed-space`, `flexible-space`;
- новые: `key` (клавиша из `SystemKey`) и `key-row` (ряд клавиш одинаковой ширины).

Размеры задаются относительно поверхности ([16-surfaces.md](16-surfaces.md)). Число без единицы — это базовая единица `u`, по умолчанию 1 % короткой стороны поверхности (у Touch Bar — высоты). Автор набора может явно выбрать `"N%w"` или `"N%h"` для отдельного значения или переопределить `u` в разделе `[units]`. Пикселей нет, поэтому одни и те же наборы работают на Touch Bar разной длины и на других поверхностях.

Привязка к состоянию: поля `*_from` (`text_from`, `value_from`, `enabled_from`, `selected_from`, `visible_from`, `symbol_from`, `canvas_from`). Узел с `visible_from = false` **не участвует** в раскладке (A7). Обработчики: `on_pressed`, `on_activate`, `on_long_pressed`, `on_value_changed`, `on_released`, `on_cancelled` — имя действия логики. Панели: `[[panels]]` с элементами, ширинами, `dismiss_on_selection` и `principal_item`, ссылки на них через `expanded_panel` и `press_and_hold_panel`.

Набор F-клавиш плагина `system` целиком декларативный. Esc добавляется одной строкой:

```toml
# plugins/system/sets/fkeys.toml
version = 1
id = "fkeys"
label = "Function keys"

[layout]
kind = "key-row"
keys = [
  # { key = "esc", width = 150 },
  { key = "f1" }, { key = "f2" }, { key = "f3" }, { key = "f4" },
  { key = "f5" }, { key = "f6" }, { key = "f7" }, { key = "f8" },
  { key = "f9" }, { key = "f10" }, { key = "f11" }, { key = "f12" },
]
```

## Поиск плагинов

Каталоги просматриваются по порядку. При совпадении `id` побеждает первый найденный плагин целиком, без слияния частей:

1. `--dev <dir>` (dev-режим, см. [11-tooling.md](11-tooling.md));
2. `$XDG_DATA_HOME/touchbar/plugins` (`~/.local/share/touchbar/plugins`);
3. `/usr/local/share/touchbar/plugins`;
4. `/usr/share/touchbar/plugins`.

Чтобы изменить поставляемый плагин, его копируют к себе и правят копию.

## Конфигурация пользователя

`$XDG_CONFIG_HOME/touchbar/config.toml` хранит только выбор, но не содержимое:

```toml
disabled = ["profile-media"]

[profile-for]                      # для всех поверхностей
"category:browser" = "my-browser-profile/browser"   # при конфликте профилей
"app:org.mozilla.firefox" = "my-firefox/firefox"

[profile-for."window:main"]        # только для одной поверхности (после v1)
"category:browser" = "my-window-profiles/browser"

[app."org.gnome.Rhythmbox3"]
categories = ["media-player"]      # заменяет автоопределение для одного приложения

[logic]
time-limit-ms = 50                 # жёсткий предел вызова логики
```

## Загрузка и проверка

Загрузчик работает в рабочем потоке. Он читает все каталоги, разбирает манифесты и файлы, проверяет ссылки, компилирует wasm (с кешем скомпилированных модулей в `$XDG_CACHE_HOME/touchbar/wasm`) и выдаёт ядру `Catalog`.

В `PluginSpec.revision` загрузчик передаёт стабильную ревизию содержимого: она меняется при изменении TOML или wasm и сохраняется при повторном чтении тех же файлов. Это не `version` манифеста и не счётчик команды `reload`. Ядро не читает файлы и не вычисляет хеши (D44).
- **Плагин с ошибкой** пропускается целиком, ошибка видна в журнале и в `status`. Остальные плагины загружаются.
- **Конфликт профилей.** Два включённых плагина объявляют профиль для одной цели (`app:…` или `category:…`), и в `profile-for` выбор не сделан. Оба профиля для этой цели не используются, а `status` и `doctor` показывают конфликт. Молча выбирать первый попавшийся нельзя.
- **Категории с одинаковым `id`** из разных плагинов объединяются: так можно дополнять списки, например браузеров.
- **Перезагрузка** по inotify в каталогах плагинов и `config.toml` или по `touchbarctl reload`. Новый каталог заменяет старый целиком, только если он загрузился. Ошибка при перезагрузке оставляет прежний каталог и не завершает сессию (A12).

## Плагины первой версии

Лежат в `plugins/` репозитория, у каждого свои тесты:

| Плагин | Содержимое |
|---|---|
| `system` | набор `fkeys`, привязка «Fn удерживается → слой `system/fkeys`» |
| `controls` | логика, наборы `volume`, `brightness`, `microphone` и панель громкости |
| `media` | логика, наборы `transport`, `timeline` |
| `browser` | логика, набор `navigation` (назад, вперёд, обновить через `desktop.shortcut`) |
| `profile-default` | профиль `default` |
| `profile-browser` | категория `browser`, профиль для `category:browser` |
| `profile-media` | категория `media-player`, профиль для `category:media-player` и вариант для `category:browser` при `media.active` |
