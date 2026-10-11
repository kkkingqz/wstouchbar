# Категории приложений и профили

## Категории

Категория — именованный метасписок приложений, например `browser` — любой браузер. Категории объявляют плагины (`categories.toml`). Правила одинаковых категорий из разных плагинов объединяются.

```toml
# plugins/profile-browser/categories.toml
[[category]]
id = "browser"
desktop-categories = ["WebBrowser"]
mime = ["x-scheme-handler/http", "x-scheme-handler/https"]
apps = []                  # явные id: для приложений без категорий, AppImage, "wmclass:…"
exclude = []               # явные исключения
```

`touchbar-apps` вычисляет упорядоченный факт `app.categories` по `app.id`:

1. **Переопределение пользователя** `[app."<id>"] categories = [...]` в `config.toml` заменяет всё остальное.
2. **Явная принадлежность** (`apps`).
3. **Автоопределение** по desktop-файлу: поле `Categories=` и, если задано `mime`, поле `MimeType=`. Учитываются только перечисленные категории freedesktop, родительские (например, `AudioVideo` для `Player`) не учитываются.
4. **`exclude`** убирает приложение из категории на любом шаге, кроме переопределения пользователя.

Порядок: сначала категории из шага 2, затем из шага 3. Внутри шага — в порядке загрузки плагинов, затем в порядке объявления. Desktop-файлы читаются из `XDG_DATA_DIRS` и `XDG_DATA_HOME`, туда попадают пакеты, Flatpak и Snap. Перечитываются по inotify. Модуль не зависит от рабочего стола.

Категории первой версии: `browser` (плагин `profile-browser`) и `media-player` (плагин `profile-media`, `desktop-categories = ["Player"]`).

## Профиль

Формат раскладки переносится из `touchbar-profile-config` исходного проекта. Добавляются поля выбора `for`, `when` и `default`, а лишний уровень `contribution` убирается: слот сразу перечисляет кандидатов.

```toml
# plugins/profile-media/profiles/media.toml
version = 1
id = "media"
for = { category = "media-player" }       # или { app = "org.videolan.VLC" }; без for — только по ссылке

[[element]]
kind = "slot"
id = "transport"
items = ["media/transport"]               # кандидаты по порядку: первый доступный

[[element]]
kind = "fixed-space"
width = 12

[[element]]
kind = "slot"
id = "timeline"
items = ["media/timeline"]

[[element]]
kind = "flexible-space"
minimum = 16
weight = 1

[[element]]
kind = "group"
id = "levels"
layout = "equal-width"
spacing = 8
element = [
  { kind = "slot", id = "volume", items = ["controls/volume"] },
  { kind = "slot", id = "brightness", items = ["controls/brightness"] },
]
```

```toml
# plugins/profile-media/profiles/browser-media.toml
version = 1
id = "browser-media"
for = { category = "browser" }
when = { "media.active" = true }
include = "profile-media/media"           # та же раскладка
```

```toml
# plugins/profile-default/profiles/default.toml
version = 1
id = "default"
default = true
# ...
```

`when` — условие над фактами: равенство, `in`, `exists`, `not`, `all`, `any`. Вложенность до 8 уровней.

## Выбор профиля

Цепочка разрешения, первый подходящий шаг побеждает:

0. **`session.locked`.** Если экран заблокирован и есть профиль с `for = { locked = true }`, используется он.
1. **Приложение.** Профили с `for = { app = <app.id> }`.
2. **Категория.** Для каждой категории из `app.categories` по порядку — профили с `for = { category = <категория> }`. Берётся первая категория, для которой профиль нашёлся.
3. **По умолчанию.** Профиль с `default = true`.

Внутри одного шага профили с выполненным `when` побеждают профиль без `when`. Для одной цели допускается не больше одного профиля без `when`. Профили с `when` для одной цели обязаны иметь разные `priority` (целое, по умолчанию 0), и при нескольких выполненных условиях побеждает больший `priority`. Обе проверки статические, при загрузке, поэтому неоднозначности во время работы не бывает. Нарушение — конфликт, который разрешается через `profile-for` (см. [04-plugins.md](04-plugins.md)). Между шагами приоритетов нет: приложение всегда сильнее категории, категория — сильнее профиля по умолчанию.

Пример: Firefox (`app.id = org.mozilla.firefox`, `app.categories = [browser]`). Своего профиля у Firefox нет. На шаге 2 для `browser` есть `browser` (без `when`) и `browser-media` (`when media.active`). Пока идёт видео, выбран `browser-media`, после паузы и истечения `media.active` — `browser`.

Смена выбранного профиля проходит через автомат `Active → Pending → Active` ядра ([03-core.md](03-core.md)). `touchbarctl explain` показывает весь путь: факты, найденные категории, кандидатов на каждом шаге, выполнение `when` и результат.

## Профили первой версии

| Профиль | Плагин | Цель |
|---|---|---|
| `default` | `profile-default` | `default = true` |
| `browser` | `profile-browser` | `category:browser` |
| `media` | `profile-media` | `category:media-player` |
| `browser-media` | `profile-media` | `category:browser` + `when media.active` |
