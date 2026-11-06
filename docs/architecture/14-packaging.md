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
| `/usr/lib/touchbar/touchbar-spawn-helper` | изоляция групп процессов spawn |
| `/usr/bin/touchbarctl` | CLI |
| `/usr/lib/systemd/system/touchbard.service` | системный юнит (запускается udev) |
| `/usr/lib/systemd/user/touchbar-session.service` | пользовательский юнит (`graphical-session.target`) |
| `/usr/lib/udev/rules.d/90-touchbar.rules` | начальная конфигурация 1 и запуск `touchbard` |
| `/usr/lib/udev/rules.d/99-wstouchbar-isolation.rules` | обязательная изоляция DRM и дигитайзера от композитора |
| `/usr/lib/tmpfiles.d/touchbar.conf` | `/run/touchbar` |
| `/usr/share/touchbar/touchbard.toml` | значения по умолчанию `touchbard` |
| `/usr/share/touchbar/plugins/*` | плагины первой версии (wasm собран при сборке пакета) |
| `/usr/share/gnome-shell/extensions/wstouchbar@kkkingqz.github.io/` | расширение GNOME |
| `/usr/share/doc/touchbar/` | документация, лицензии |

`/etc/touchbar/touchbard.toml` пакет не создаёт: это файл администратора.

## Дерево этапа 4

`xtask install` создаёт отдельное staging-дерево: `--destdir` обязателен,
`--prefix` по умолчанию `/usr`. Команда собирает четыре бинарника и плагины,
учитывая Cargo target directory из окружения. По умолчанию используется
`--profile release`; для локальной проверки можно выбрать `--profile dev`
и использовать существующий кеш сборки. Службы и правила не активируются.
Расширение GNOME в дерево этапа 4 не входит.

В корне дерева записывается `.touchbar-install-manifest.json` версии 1:
логический prefix, относительные пути, SHA-256 и права файлов. Абсолютных
путей к каталогу сборщика в манифесте нет. Этот служебный файл не включается
в payload будущего пакета. Бинарники получают 0755, данные — 0644, каталоги —
0755. Предварительная сборка публикуется целиком; непустое чужое дерево,
изменённые установленные файлы и дополнительные файлы не заменяются.

```sh
cargo xtask install --profile dev --prefix=/usr --destdir=target/stage
cargo xtask uninstall --manifest target/stage/.touchbar-install-manifest.json
```

`uninstall` работает с этим staging-деревом. Перед любым удалением проверяются
весь манифест, пути, хеши и права. Изменённый файл приводит к отказу; посторонние
файлы сохраняются. Пути разрешаются относительно открытого корня с запретом
символьных ссылок и выхода за его пределы. Prefix может задавать другую
раскладку сборочного дерева; для штатной установки и поиска ресурсов используется
`/usr`. Ни install, ни uninstall не вызывают systemctl или udevadm.

Системный юнит сохраняет `/run/touchbar` после остановки, включая recovery и
маркер autodim. Явный `BindReadOnlyPaths=/sys` обеспечивает read-only sysfs:
на проверенной systemd 259 одного ProtectKernelTunables недостаточно при
уже смонтированном API sysfs. Перед запуском загружается `hid_appletb_kbd`; новая файловая
namespace разрешает запись в `/sys/devices` и параметры только этого модуля
(D129). USB/HID/подсветка при повторном появлении могут получить другие пути,
поэтому исключение на одну старую директорию устройства недостаточно.
`appletbdrm` по-прежнему загружает демон после проверки правила изоляции.
Доступ к символьным устройствам отдельно ограничен DevicePolicy/DeviceAllow.
Минимальный CapabilityBoundingSet проверяется аппаратным стендом до закрытия H16.

Офлайн-проверка файлов (без запуска служб):

```sh
cargo xtask test-env packaging
```

Требуются systemd-analyze, modprobe и udevadm. При проверке пути исполняемых
файлов в юнитах перемещаются на временные исполняемые заглушки; настройки
изоляции и правила udev не меняются. Аппаратную работоспособность такая проверка
не подтверждает.

Отдельный ignored-тест `env_systemd_sysfs_mount_exceptions_are_effective`
проверяет фактические флаги монтирований в пользовательском systemd-юните
с настройками sysfs из шаблона. Ему нужны работающий user manager, user
namespaces и Python 3. Он только читает метаданные, не открывает устройства,
не запускает демон и не включён в офлайн-набор `test-env packaging`.

## Поведение установки пакета (следующий этап упаковки)

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

- **Rust:** stable, MSRV фиксируется в `rust-toolchain.toml`. Цели: `x86_64-unknown-linux-gnu`, а для плагинов `wasm32-unknown-unknown` (тоже в `rust-toolchain.toml`) с превращением в компонент библиотекой `wit-component` внутри `xtask` и `touchbar-wasm-fixtures`.
- **Системные зависимости сборки:** `libpipewire-0.3-dev`, `libegl-dev`, `libfontconfig-dev`; нужны pkg-config и clang/libclang для привязок PipeWire. DRM/udev не требуют соответствующих dev-пакетов (D111); EGL/GLES и необязательный libgbm используются во время GPU-проверок.
- **Локальная сборка на машине пользователя** — в distrobox `touchbar-build` (на хосте нет компилятора).

## wsconfig

wsconfig ставит тот же `.deb`:
- Пакет скачивается из Releases по версии и sha256, как xremap.
- В wsconfig остаются только личные настройки: `config.toml`, `bindings.toml`, свои плагины в `~/.local/share/touchbar/plugins`, включение расширения.
- `ws-touchbar-fn` и его юнит удаляются из wsconfig: мост Fn и затемнение делает `touchbard`. Удаление идёт тем же изменением, что и установка пакета, чтобы у Touch Bar не было двух владельцев.
- Свои правила нативного режима в wsconfig (`90-touchbar-native.rules`, blacklist `appletbdrm`) проверяются на совместимость: `touchbard` загружает `appletbdrm` сам, в обход blacklist. Лишние правила удаляются.
