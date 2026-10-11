# Привязки нажатий

Привязка — правило «триггер → действие». Привязки задаются декларативно, синтаксис и автоматы общие (`touchbar-bindings`) для двух мест:

| Где | Файл | Действия | Кто разбирает |
|---|---|---|---|
| Сессия | `bindings.toml` в плагинах и `$XDG_CONFIG_HOME/touchbar/bindings.toml` | слои | `touchbar-core` по `KeyEvent` от `touchbard` |
| `touchbard` | `/usr/share/touchbar/touchbard.toml`, `/etc/touchbar/touchbard.toml` | режимы (recovery) | сам `touchbard` по evdev, независимо от сессии |

Привязки только **наблюдают** клавиши, а не перехватывают их: нажатия доходят до системы как обычно.

## Триггер

```toml
trigger = { key = "fn", gesture = "while-held" }
trigger = { keys = ["fn", "esc"], gesture = "hold", ms = 3000 }   # аккорд
trigger = { key = "fn", gesture = "multi-tap", count = 2, within-ms = 300 }
trigger = { sequence = [ { key = "fn", gesture = "tap" }, { key = "esc", gesture = "tap" } ], within-ms = 800 }
```

| Жест | Срабатывает | Параметры |
|---|---|---|
| `press` | при нажатии | — |
| `release` | при отпускании | — |
| `while-held` | действует, пока клавиша или аккорд удерживается | — |
| `hold` | один раз после удержания дольше `ms` | `ms` |
| `tap` | при отпускании, если удержание было короче `ms` и в это время не нажимались другие клавиши | `ms` (по умолчанию 300) |
| `multi-tap` | `count` нажатий типа `tap` за `within-ms` | `count`, `within-ms` |
| `sequence` | шаги по порядку за `within-ms` | шаги — любые жесты, кроме `while-held` |

`key` — имя из закрытого списка `WatchKey` ([02-protocol.md](02-protocol.md)): `fn`, `esc`, модификаторы, `f1`–`f12`. В аккорде `keys` триггер считается начавшимся, когда нажата последняя клавиша аккорда, а остальные ещё удерживаются.

Позже можно добавить триггеры касанием самой полосы (`touch`: свайп двумя пальцами, удержание у края), автоматы будут те же.

## Действия

**Сессия:**

| Действие | Смысл |
|---|---|
| `{ layer = "plugin/set" }` | показать набор слоем поверх профиля, пока действует триггер `while-held`, или до следующего срабатывания для других жестов |
| `{ toggle-layer = "plugin/set" }` | включить или выключить слой |
| `{ clear-layers = true }` | снять все слои |

**`touchbard`:**

| Действие | Смысл |
|---|---|
| `"native"` | включить recovery |
| `"display"` | выключить recovery |
| `"toggle-recovery"` | переключить |

Условие `when` в `touchbard` поддерживает только `mode` и `recovery`, а в сессии — факты, как в профилях.

## По умолчанию

```toml
# plugins/system/bindings.toml
[[binding]]
id = "fn-layer"
trigger = { key = "fn", gesture = "while-held" }
action  = { layer = "system/fkeys" }
```

```toml
# /usr/share/touchbar/touchbard.toml
[[binding]]
id = "recovery"
trigger = { key = "fn", gesture = "hold", ms = 8000 }
action  = "toggle-recovery"
```

Удержание Fn сначала показывает F-клавиши, а через 8 секунд переключает recovery. Это поведение исходного проекта, и оно работает на любой модели с T2, включая модели без физического Esc.

## Переопределение

Пользовательский `bindings.toml` и `/etc/touchbar/touchbard.toml` заменяют привязки с тем же `id` и добавляют новые. Чтобы отключить привязку, достаточно `{ id = "fn-layer", disabled = true }`.

## Автомат привязки

У каждой привязки свой автомат `Idle → Armed → (Active) → Idle`, чистый, с таймерами через `now`. Несколько привязок на одни клавиши независимы: каждая срабатывает по своему условию. Проверка при загрузке запрещает только одинаковые триггеры с разными действиями без разных `when`.

При смене режима, отзыве и переподключении к `touchbard` все автоматы сессии сбрасываются в `Idle`, а слои `while-held` снимаются. Автоматы `touchbard` при этом не сбрасываются: recovery не зависит от сессии.
