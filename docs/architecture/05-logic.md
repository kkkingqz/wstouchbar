# Логика плагинов (wasm)

## Модель

Логика — это чистая функция «событие → обновление». Модуль ничего не импортирует, кроме логирования: нет ни WASI, ни брокера, ни системных часов. Всё, что меняет мир, модуль описывает данными в обновлении, а выполняет это оболочка. Результаты возвращаются модулю новыми событиями.

```
событие ──▶ logic.handle(event, ctx) ──▶ update { state, requests, cancel, subscriptions, presentation }
```

Следствия:
- Логику можно тестировать без процессов, D-Bus и железа. Тест подаёт события и сравнивает обновления.
- Политики отмены, объединения и таймаутов одинаковы для всех плагинов, их применяет ядро.
- Логика не может заблокировать цикл ожиданием ввода-вывода: она выполняется только между событиями.

## Экземпляры

- **Компиляция.** Модуль компилируется один раз на плагин, скомпилированный модуль кешируется на диске.
- **Экземпляры.** Отдельный экземпляр wasm создаётся на каждый экземпляр набора в профиле. `controls` с `logic_config = "volume"` и `controls` с `logic_config = "microphone"` — два независимых экземпляра со своей памятью.
- **Панели.** Виджеты `PanelSpec` вызывают ту же логику и используют состояние владельца. Хост сохраняет физический захват и якорь приватно вместе с вызовом; WIT не меняется (D48–D49).
- **Поток.** Вызовы выполняются в главном потоке сессии.

## Контракт (WIT)

Это черновик. Точные типы закрепляются на этапе 2 (см. [15-roadmap.md](15-roadmap.md)), файл — `wit/logic.wit`.

```wit
package wstouchbar:logic@1.0.0;

interface types {
    enum scalar-kind { boolean, number, text }
    variant scalar { boolean(bool), number(f64), text(string) }

    /// Поле состояния. Записи нужны для `repeat`, canvas для `canvas_from`.
    variant field-kind { scalar(scalar-kind), records(list<tuple<string, scalar-kind>>), canvas }
    record field { name: string, kind: field-kind, persist: bool }

    type record-value = list<tuple<string, scalar>>;
    variant value { scalar(scalar), records(list<record-value>), canvas(list<canvas-command>) }
    type state-patch = list<tuple<string, value>>;

    // canvas-command — перенос из ui WIT исходного проекта (линии, пути, текст, градиенты)

    enum action-kind { pressed, long-pressed, activated, released, cancelled, value-changed, selected }
    record action-description { kinds: list<action-kind>, value: option<scalar-kind> }
    record description {
        actions: list<tuple<string, action-description>>,
        state: list<field>,
    }

    enum origin { physical, synthetic }
    enum cancel-reason { hidden, layer, presentation, profile, layout, mode, revoked, fault, hardware, surface-removed }
    record action-event {
        action: string,
        kind: action-kind,
        value: option<scalar>,
        record-key: option<string>,
        origin: origin,
        cancel-reason: option<cancel-reason>,
    }

    record context {
        surface-kind: string,         // touchbar | window | headless
        slot-width: f32,              // ширина слота в базовых единицах u
        representation: string,       // текущий вариант responsive
        color-scheme: string,         // light | dark
        accent: option<string>,
        locale: string,
    }

    variant event {
        action(action-event),
        shown,
        hidden,
        context-changed(context),
        presentation-closed(string),  // id панели, закрытой ядром
        effect-done(effect-result),
        subscription-data(subscription-data),
        subscription-lost(subscription-lost),
    }

    record call-context { now-ms: u64 }
}

interface effects {
    // Подробно — в 06-effects.md
    variant effect { /* audio, media, desktop, key, open-uri, spawn, dbus-call, timer */ }
    enum lifetime { until-hidden, committed }
    record request {
        tag: string,                  // логика сама сопоставляет результат; ID хоста она не видит
        effect: effect,
        lifetime: option<lifetime>,   // по умолчанию из каталога действий
        coalesce: option<string>,     // ключ объединения, выполняется последний запрос
        timeout-ms: option<u32>,
    }
    variant subscription-kind { /* audio-watch, media-watch, desktop-fact, interval, spawn-lines, dbus-signal */ }
    record subscription { tag: string, kind: subscription-kind, background: bool }
    // effect-result, subscription-data, subscription-lost — типизированные варианты по видам
}

interface presentation {
    enum placement { anchored, region, full }
    enum lifecycle { transient, persistent }   // transient — пока держится касание-якорь
    variant command { open(tuple<string, placement, lifecycle>), close }
}

world logic {
    use types.{description, event, call-context, state-patch};
    use effects.{request, subscription};
    use presentation.{command};

    record update {
        state: state-patch,
        requests: list<request>,
        cancel: list<string>,                     // теги запросов
        subscriptions: option<list<subscription>>, // none = без изменений, some = желаемое множество целиком
        presentation: option<command>,
    }

    import log: func(level: u8, message: string);

    export describe: func() -> description;
    export init: func(config: string, persisted: state-patch, ctx: call-context) -> result<update, string>;
    export handle: func(event: event, ctx: call-context) -> result<update, string>;
}
```

В WIT нет рекурсивных типов. Поэтому значения D-Bus и прочие деревья передаются строкой в фиксированном формате (сигнатура D-Bus и значение в JSON, см. [06-effects.md](06-effects.md)).

## Правила вызова

- **`describe`** вызывается один раз на модуль. Ядро проверяет, что все действия и поля состояния, на которые ссылаются наборы, объявлены и подходят по типам.
- **`init`** получает `logic_config` экземпляра и сохранённые поля (`persist = true`). Ядро вызывает его после проверки `LogicDescribed`, через `LogicCall::Init` с обычным `CallId`. Только успешный ответ переводит экземпляр в `Hidden`. `LogicCall::Handle` вызывает `handle`; оба вида вызова занимают один общий слот. Это тип хоста, варианты `event` в WIT не меняются (D41).
- **`handle`** вызывается на каждое событие. События одного экземпляра идут строго последовательно.
- **`Err(string)`** передаётся как `LogicError` (D45). Для `handle` обновление отбрасывается, ошибка пишется в журнал, экземпляр продолжает работать и освобождает слот для следующего события. Для `init` экземпляр переходит в `Faulted`, повтор идёт с backoff.
- **Недопустимое содержимое** обновления отбрасывается по частям с записью в журнал, весь вызов не отвергается:
  - запрос с неизвестным видом или параметрами вне допустимых пределов;
  - `presentation.open` в ответ на `synthetic`;
  - поле, которого нет в схеме.

## Лимиты и сбои

Константы хранятся в `touchbar-logic-types` и общие для хоста и SDK (A13).

| Лимит | Значение по умолчанию |
|---|---|
| Время вызова | мягкий предел 5 мс (предупреждение в журнале), жёсткий 50 мс (epoch interruption → сбой) |
| Память экземпляра | 16 МиБ |
| Размер обновления | 256 КиБ, до 64 запросов, до 32 подписок |
| Текст в состоянии | 4 КиБ на поле, 1000 записей на поле |
| Сохраняемое состояние | 64 КиБ на экземпляр |

Сбой — это trap, превышение жёсткого предела времени или памяти. Он переводит экземпляр в `Faulted` (см. [03-core.md](03-core.md)). Повтор идёт с backoff, состояние начинается заново с `init` и сохранённых полей. Хост из-за плагина не завершается никогда.

Ошибка загрузки или выполнения `describe`, пока `CallId` ещё нет, передаётся как `InstanceFault` (D45) и повторяется с тем же backoff. Несовместимость описания с набором диагностируется отдельно: неизвестное действие, неподдерживаемый вид события, несовместимый тип поля или невалидная схема переводят экземпляр в `Faulted` без таймерных повторов до перезагрузки каталога. Привязки `record.*` внутри повторяемых элементов сверяются со схемой записи. `ValueChanged` требует числового значения действия.

## Сохраняемое состояние

Поля с `persist = true` сохраняются по стабильному ключу: плагин-владелец набора (не поставщик общей логики), имя поверхности и область размещения (D54). Для профиля путь остаётся `$XDG_STATE_HOME/touchbar/<plugin>/<surface>/<slot>.json`; для слоя — `$XDG_STATE_HOME/touchbar/<plugin>/<surface>/layers/<set>.json`. Области независимы, даже если идентификаторы слота и набора совпадают.

`CreateInstance.persist_key` позволяет хосту прочитать файл; он возвращает поля в `LogicDescribed.persisted`. Ядро проверяет описание до заполнения кеша, отбрасывает неизвестные, несохраняемые, несовместимые и превышающие лимиты поля, затем передаёт результат в `init`. Если в ядре уже есть состояние этого ключа, оно имеет приоритет над загруженным с диска (D55), в том числе после пересоздания по ревизии или переподключения поверхности.

Принятые изменения объединяются. Запись выдаётся через 1 с после последнего изменения сохраняемых значений; повтор прежнего значения задержку не продлевает. `Output::Persist` содержит только поля, отличающиеся от загруженных или последней выданной записи. Возврат всех полей к прежним значениям снимает таймер. Уничтожение экземпляра не отменяет запись: хост выполняет её по стабильному ключу. Кеш и таймеры входят в контрольную точку ядра. Полный набор сохраняемых полей ограничен 64 КиБ в JSON; превышение диагностируется и не заменяет сохранённые значения.

## SDK

`touchbar-logic-sdk` (Rust) собирается под `wasm32-unknown-unknown`, где у `std` нет импортов WASI. `xtask` превращает модуль в компонент (`wasm-tools component new`). SDK даёт:
- типизированные помощники для обновлений и запросов;
- разбор результатов модулей `audio`, `media` и `desktop`;
- макрос экспорта;
- тестовую обвязку для запуска модуля с последовательностью событий через `touchbar-testkit`.
