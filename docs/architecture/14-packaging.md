# Сборка и установка

## Принцип

Штатная установка не зависит от wsconfig. Единственный источник правды о том, что и куда ставится, — `cargo xtask install`. Все форматы пакетов строятся из его дерева.

```bash
cargo xtask install --prefix=/usr --destdir=target/stage
```

## Раскладка файлов

| Путь | Что |
|---|---|
| `/usr/lib/touchbar/touchbard` | root-демон |
| `/usr/lib/touchbar/touchbar-session` | сессия |
| `/usr/bin/touchbarctl` | CLI |
| `/usr/lib/systemd/system/touchbard.service` | системный юнит (запускается udev) |
| `/usr/lib/systemd/user/touchbar-session.service` | пользовательский юнит (`graphical-session.target`) |
| `/usr/lib/systemd/system-sleep/touchbar` | `touchbarctl sleep pre\|post` |
| `/usr/lib/udev/rules.d/90-touchbar.rules` | запуск `touchbard`, права устройств |
| `/usr/lib/sysusers.d/touchbar.conf` | группа `touchbar` |
| `/usr/lib/tmpfiles.d/touchbar.conf` | `/run/touchbar` |
| `/usr/share/touchbar/touchbard.toml` | значения по умолчанию `touchbard` |
| `/usr/share/touchbar/plugins/*` | плагины первой версии (wasm собран при сборке пакета) |
| `/usr/share/gnome-shell/extensions/wstouchbar@kkkingqz.github.io/` | расширение GNOME |
| `/usr/share/doc/touchbar/` | документация, лицензии |

`/etc/touchbar/touchbard.toml` пакет не создаёт: это файл администратора.

## Поведение установки

- **Сразу после установки ничего не меняется.** Touch Bar остаётся в нативном режиме, пока пользователь не войдёт и сессия не подключится.
- **Пакет включает** `touchbar-session.service` глобально (`systemctl --global enable`) и перечитывает правила udev. Расширение GNOME каждый пользователь включает сам через `touchbarctl setup`. Без расширения сессия работает с бэкендом `null` и подсказывает в журнале и `status` запустить `setup`.
- **Конфликты:** `tiny-dfr`, старые пакеты `touchbar` и `touchbar-git`. Если найдены свои правила или параметры `hid-appletb-kbd`, `doctor` о них предупреждает.
- **Удаление (`purge`).** `touchbard` останавливается, возвращает драйверу `autodim` и нативный режим. Глобальное включение юнита снимается.

## Форматы v1

| Формат | Как |
|---|---|
| `.deb` | `cargo xtask deb` (из staging-дерева, `dpkg-deb`), сборка в контейнере Ubuntu 26.04 |
| PKGBUILD (AUR) | `packaging/aur/wstouchbar` и `wstouchbar-git`, тот же `xtask install` |

RPM (Fedora через COPR) — позже, если появится спрос.

## Каналы

- **Сейчас:** GitHub Releases. Ассеты `.deb` и `SHA256SUMS`, установка `apt install ./wstouchbar_X.Y.Z_amd64.deb`.
- **Цель:** apt-репозиторий t2linux, когда проект стабилизируется.

## Сборка

- **Rust:** stable, MSRV фиксируется в `rust-toolchain.toml`. Цели: `x86_64-unknown-linux-gnu`, а для плагинов `wasm32-unknown-unknown` с превращением в компонент (`wasm-tools component new` внутри `xtask`).
- **Системные зависимости сборки:** `libpipewire-0.3-dev`, `libdrm-dev`, `libgbm-dev`, `libegl-dev`, `libudev-dev`, `libfontconfig-dev`.
- **Локальная сборка на машине пользователя** — в distrobox `touchbar-build` (на хосте нет компилятора).

## wsconfig

wsconfig ставит тот же `.deb`:
- Пакет скачивается из Releases по версии и sha256, как xremap.
- В wsconfig остаются только личные настройки: `config.toml`, `bindings.toml`, свои плагины в `~/.local/share/touchbar/plugins`, включение расширения.
- `ws-touchbar-fn` и его юнит удаляются из wsconfig: мост Fn и затемнение делает `touchbard`. Удаление идёт тем же изменением, что и установка пакета, чтобы у Touch Bar не было двух владельцев.
- Свои правила нативного режима в wsconfig (`90-touchbar-native.rules`, blacklist `appletbdrm`) проверяются на совместимость: `touchbard` загружает `appletbdrm` сам, в обход blacklist. Лишние правила удаляются.
