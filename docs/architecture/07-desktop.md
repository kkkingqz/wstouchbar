# API рабочего стола

Ядро не знает о GNOME, Hyprland и других окружениях. Оно видит только **факты** и выдаёт **команды рабочего стола** как разновидность действий. Всё, что зависит от окружения, находится за API `touchbar-desktop`. Бэкенды встраиваются в процесс сессии.

```
touchbar-core ◀─ факты ── оболочка ◀─ BackendEvent ── бэкенд (touchbar-desktop-gnome | -null | позже -hyprland)
              ── desktop.* ▶ оболочка ── DesktopCommand ▶
```

## Трейт

```rust
pub trait DesktopBackend {
    fn name(&self) -> &'static str;
    /// Запуск: бэкенд сам подключается и переподключается, сообщая о себе событиями.
    fn start(&mut self, events: Sender<BackendEvent>) -> Result<()>;
    fn command(&self, command: DesktopCommand) -> CommandFuture; // Result<(), DesktopError>
}

pub enum BackendEvent {
    Ready(Capabilities),
    Fact(FactName, Option<FactValue>),   // None — факт отсутствует
    Lost(String),
}
```

Граница проведена так, чтобы позже бэкенд можно было вынести в отдельный процесс, не меняя ядро: события и команды — простые сериализуемые типы.

**Выбор бэкенда:** параметр `desktop = "auto" | "gnome" | "null"` в `config.toml`. В режиме `auto` выбирается `gnome`, если `XDG_CURRENT_DESKTOP` содержит `GNOME`, иначе `null`.

## Факты

| Факт | Тип | Смысл |
|---|---|---|
| `app.id` | текст | id desktop-файла приложения в фокусе (`org.mozilla.firefox`). Если его нет — `wmclass:<class>`. Без фокуса (рабочий стол) факта нет. Собственные окна wstouchbar (будущая поверхность `window`) в факт не попадают |
| `app.fullscreen` | логический | окно в фокусе развёрнуто на весь экран |
| `session.locked` | логический | экран заблокирован |
| `appearance.scheme` | текст | `light` или `dark` |
| `appearance.accent` | текст | акцентный цвет (`#rrggbb`) |
| `appearance.font` | текст | шрифт интерфейса (семейство и размер) |
| `workspaces` | записи | `{id, name, active, windows}` |
| `workspace.active` | текст | id активного рабочего стола |
| `display.brightness` | число | яркость экрана 0.0–1.0 |

`app.id` бэкенд обязан нормализовать именно так. Тогда один и тот же профиль работает в любом окружении. `app.categories` вычисляет не бэкенд, а `touchbar-apps` по `app.id` (см. [08-profiles.md](08-profiles.md)).

## Команды

`shortcut(keys, app)`, `key(SystemKey, phase)` (для поверхностей без своего ввода клавиш, [16-surfaces.md](16-surfaces.md)), `brightness.set(level)`, `workspace.activate(id)`, `app.launch(id)`. Соответствующие действия плагинов описаны в [06-effects.md](06-effects.md).

## Возможности

`Ready(Capabilities)` перечисляет факты и команды, которые поддерживает бэкенд. Неподдерживаемый факт просто отсутствует. Неподдерживаемая команда возвращает `unsupported`, и плагин деградирует: скрывает кнопку или делает её неактивной.

## Жизненный цикл бэкенда

```
Connecting ──Ready──▶ Ready ──Lost──▶ Connecting (backoff 1, 2, 4… 60 с)
```

При `Lost` оболочка снимает все факты бэкенда (`Fact(_, None)`), и профили откатываются к тем, которым эти факты не нужны. Постоянные ошибки, например отсутствие расширения, не вызывают частых повторов: бэкенд ждёт появления D-Bus имени (`NameOwnerChanged`), а не опрашивает его (A9).

## Бэкенд GNOME

Состоит из двух частей: Rust-бэкенда `touchbar-desktop-gnome` и собственного расширения GNOME Shell из `gnome-extension/` этого репозитория. Window Monitor Pro и портал RemoteDesktop больше не нужны.

### Расширение

- **UUID:** `wstouchbar@kkkingqz.github.io`. Ставится системно в `/usr/share/gnome-shell/extensions/`, включается командой `touchbarctl setup`.
- **D-Bus:** имя `io.github.kkkingqz.WsTouchbar.Shell` на сессионной шине, объект `/io/github/kkkingqz/WsTouchbar/Shell`, интерфейс `io.github.kkkingqz.WsTouchbar.Shell1`.

| Член | Вид | Смысл |
|---|---|---|
| `Version`, `Capabilities` | свойства | версия интерфейса, список поддерживаемого |
| `GetState() → a{sv}` | метод | все факты разом (при подключении) |
| `StateChanged(a{sv})` | сигнал | изменившиеся факты |
| `Shortcut(as keys, s app) → ()` | метод | нажать сочетание, если `app` пустой или совпадает с приложением в фокусе. Пока экран заблокирован, отвергается (как и `LaunchApp`) |
| `ActivateWorkspace(s id)` | метод | |
| `LaunchApp(s id)` | метод | |

Источники в GNOME Shell:

| Что | Откуда |
|---|---|
| `app.id` | `Shell.WindowTracker` → `Shell.App.get_id()` без `.desktop`, запасной вариант — WM class |
| `app.fullscreen` | `Meta.Window.is_fullscreen()` |
| `session.locked` | `Main.sessionMode.isLocked` |
| `appearance.*` | `org.gnome.desktop.interface` (`color-scheme`, `accent-color`, `font-name`) |
| `workspaces` | `global.workspace_manager` |
| нажатие сочетаний | виртуальное устройство Clutter (`create_virtual_device(KEYBOARD_DEVICE)`), без портала и диалога |

`display.brightness` бэкенд берёт напрямую из `org.gnome.Shell.Brightness`, этот интерфейс есть в самом GNOME Shell.

Расширение минимальное, около 300 строк JS, и не содержит логики Touch Bar: только факты и команды. Поддерживаемые версии GNOME Shell перечислены в `metadata.json`, целевая — версия из Ubuntu 26.04.

### Модель угроз

Методы расширения доступны любому процессу пользователя без песочницы на сессионной шине. Такие процессы и так находятся внутри границы доверия пользователя. Приложения Flatpak к этому имени доступа не имеют, если им явно не разрешили (xdg-dbus-proxy). Дополнительно расширение принимает изменяющие методы только от соединения, которое владеет именем `io.github.kkkingqz.WsTouchbar.Session`. Это имя занимает сессия. Бэкенд, в свою очередь, принимает сигналы только от владельца имени расширения.

## Другие окружения

Бэкенды для Hyprland и Omarchy делаются позже по тому же трейту: факты из IPC Hyprland (`activewindow`, `workspace`), команды через `hyprctl dispatch`. Ни ядро, ни плагины при этом не меняются.
