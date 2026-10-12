# План: этап 2 — логика

Этап 1 закрыт коммитом `1079add`, итоги — в [приёмке](../validation/stage-1.md). Открытые вопросы из [decisions.md](../architecture/decisions.md) этап 2 не блокируют.

## Цель

Закрепить контракт `wit/logic.wit` версии 1.0.0, написать runtime на wasmtime, SDK для авторов логики и перенести плагин `controls`.

Этап готов, когда `controls` проходит тесты логики в testkit на двух уровнях:
- **модуль:** события подаются компоненту напрямую, проверяются обновления;
- **сценарии:** настоящее ядро, настоящий runtime с компонентом `controls`, поддельные исполнитель и рендер.

На этом этапе нет настоящих исполнителей (PipeWire, рабочий стол), рендера, бинарника сессии и загрузчика каталогов плагинов. Ядро не меняется, кроме мелочей, найденных по ходу работ.

## Уточнения архитектуры

Найдены при планировании, согласованы и внесены в документы архитектуры и журнал решений (D66–D72).

1. **Формат наборов переезжает в этап 2 (D66).** `touchbar-sets` описывает схему TOML наборов целиком, включая атрибуты раскладки, и манифест `plugin.toml`. Здесь же преобразование в `SetSpec` и проверки. Построение дерева UI из набора и состояния остаётся в этапе 3. Причина: тесты `controls` должны проверять его настоящие файлы, а ядро — сверять `describe` с настоящими наборами. Описывать наборы ещё раз в Rust — значит держать две копии, которые разойдутся.
2. **SDK работает с типами `touchbar-model` (D67).** Автор логики пишет `impl Logic` на тех же `LogicEvent`, `LogicUpdate`, `Effect`, `StatePatch`, что видят ядро и testkit. Перевод в WIT и обратно скрыт в SDK (гость) и в runtime (хост). Один словарь на весь проект, а логику можно проверять и без wasm.
3. **Время задаёт вызывающий (D68).** `Runtime::call(now, …)` передаёт `call-context.now-ms`, других часов у модуля нет. Движок настроен детерминированно: канонизация NaN, без потоков, детерминированный relaxed SIMD, без WASI. Одинаковые входы дают одинаковые обновления.
4. **Версия контракта (D69)** проверяется по точному имени экспортируемого интерфейса `wstouchbar:logic/api@1.0.0`, затем по его типам. Функции `describe/init/handle` и типы контракта объявлены в `api`. Каждая будущая поддерживаемая версия интерфейса получит свои биндинги runtime, переводящиеся в те же типы модели. Другая версия отвергается даже с идентичными сигнатурами. Неподдерживаемая версия, несовместимые типы или любой импорт, кроме `log`, дают `InstanceFault` при загрузке.
5. **Кеш компиляции — встроенный кеш wasmtime (D70)** в `$XDG_CACHE_HOME/touchbar/wasm`. Своя сериализация потребовала бы `unsafe` (`Component::deserialize`), а в workspace действует `unsafe_code = "forbid"`.
6. **Разделение проверок (D71).** Runtime отвечает за то, что видно только ему: время вызова, память, общий размер обновления и вызова (нарушение — `LogicFault`). Перевод из WIT отбрасывает непредставимые элементы с предупреждением, например повторные ключи записи или невалидные идентификаторы. Остальное ядро проверяет поэлементно, как на этапе 1.
7. **Лимиты в одном месте (A13, D71).** Константы переезжают в `touchbar-logic-types`, а `CoreConfig::default()` берёт их оттуда. Формат `CoreConfig` и версия контрольной точки не меняются.
8. **Открытие панелей в `controls` (D72)** через встроенные `expanded_panel` и `press_and_hold_panel` ([03-core.md](../architecture/03-core.md)), без `presentation.open` из логики. Логика отвечает только за значения и действия.

## Состав

```
wit/logic.wit                    контракт, package wstouchbar:logic@1.0.0
crates/
  touchbar-logic-types/          лимиты, бюджет обновления, оценка размера; зависит только от touchbar-model
  touchbar-logic-sdk/            гость: трейт Logic, export!, построители, журнал; wit-bindgen
  touchbar-logic-runtime/        хост: wasmtime, биндинги, перевод WIT ↔ модель, лимиты, кеш, PersistStore
  touchbar-sets/                 формат plugin.toml и наборов → SetSpec
  touchbar-wasm-fixtures/        build.rs: собирает плагины и тестовые модули в компоненты
  touchbar-testkit/              + WasmLogic, ModuleTester, загрузка плагина из каталога
plugins/
  controls/                      plugin.toml, sets/*.toml, src/ (логика), tests/
tests/logic-fixtures/            тестовые модули: echo, trap, hang, oom, err, budget, log, persist, version, wasi
```

Новые внешние зависимости:
- `wasmtime` с возможностями `component-model`, `cranelift`, `cache`, `runtime`, `std`, без `wasmtime-wasi`;
- `wit-bindgen` для гостя и `wit-component` для упаковки в компонент в build.rs;
- `toml`.

Версии фиксируются на начало работ в `Cargo.lock`. В `rust-toolchain.toml` добавляется `targets = ["wasm32-unknown-unknown"]`.

Код, который генерируют `wasmtime::component::bindgen!` и `wit_bindgen::generate!`, содержит `unsafe`. Поэтому в `touchbar-logic-runtime`, `touchbar-logic-sdk` и плагинах действует `unsafe_code = "deny"` с `allow` только на модуле биндингов. В остальных крейтах остаётся `forbid`.

## Задачи

Каждая задача заканчивается коммитом с зелёными `fmt`, `clippy` и тестами. Размеры — как на этапе 1: S до дня, M 1–3 дня, L больше.

| # | Задача | Размер |
|---|---|---|
| L1 | Каркас: workspace, toolchain, lints, CI | S |
| L2 | `wit/logic.wit` 1.0.0 | M |
| L3 | `touchbar-logic-types` | S |
| L4 | `touchbar-logic-sdk` | M |
| L5 | `touchbar-wasm-fixtures` и тестовые модули | M |
| L6 | Runtime: загрузка, экземпляры, вызовы | L |
| L7 | Runtime: лимиты и сбои | M |
| L8 | Runtime: сохраняемое состояние | S |
| L9 | `touchbar-sets` | L |
| L10 | testkit: `WasmLogic`, `ModuleTester`, плагин из каталога | M |
| L11 | `controls`: наборы и логика | M |
| L12 | `controls`: тесты | M |
| L13 | Приёмка этапа | S |

L9 не зависит от L2–L8 и может идти параллельно с ними сразу после L1.

### L1. Каркас

- Новые крейты в workspace. `plugins/controls` и `tests/logic-fixtures/*` тоже входят в workspace: логика компилируется и под хост, поэтому `cargo test --workspace` проверяет её без wasm.
- `rust-toolchain.toml` с целью `wasm32-unknown-unknown`: rustup ставит её сам, `cargo test` на свежем клоне ничего не требует.
- Исключения для `unsafe_code` в крейтах с биндингами.
- CI остаётся тем же: `cargo test --workspace` сам собирает wasm через `touchbar-wasm-fixtures`. Кеш `Swatinem/rust-cache` охватывает и каталог сборки фикстур.

### L2. Контракт `wit/logic.wit`

Черновик из [05-logic.md](../architecture/05-logic.md) превращается в окончательный файл. Каждый тип WIT один к одному соответствует типу `touchbar-model`, таблица соответствия записывается в 05-logic.md.

- **Скаляры и состояние:** `scalar`, `scalar-kind`, `field-kind`, `field`, `record-value`, `value`, `state-patch`, `canvas-command` и сопутствующие типы — перенос из ui WIT исходного проекта по `CanvasCommand` модели.
- **Описание:** `action-kind`, `action-description`, `description`.
- **События:** `action-event` с `origin` и `cancel-reason`, `context`, `event`. Варианты `effect-done`, `subscription-data` и `subscription-lost` — записи с тегом.
- **Действия:** `effect` — по варианту на каждый `Effect` каталога v1 с параметрами из [06-effects.md](../architecture/06-effects.md), включая `spawn-spec`, `dbus-call` и `system-key`. Также `lifetime`, `request`, `effect-result`, `effect-error`, `effect-outcome`.
- **Подписки:** `subscription-kind` по `SubscriptionKind`, `subscription`, `subscription-data` (`audio-state`, `media-state`, `fact(option<fact-value>)`, `tick`, `line`, `dbus`).
- **Панели:** `placement`, `lifecycle`, `command` (`open` — запись с `panel`, `placement`, `lifecycle`).
- **Журнал:** `log(level: log-level, message: string)`, где `log-level` — перечисление вместо `u8`.
- **Экспорт:** интерфейс `api` с `describe`, `init(config, persisted, ctx)`, `handle(event, ctx)` и типами контракта, включая `update`. World `logic` экспортирует `api` под полным именем `wstouchbar:logic/api@1.0.0`; отдельные функции world не экспортируются. Типы находятся в `api`, чтобы не создавать транзитивных импортов вспомогательных интерфейсов.

`surface-kind`, `color-scheme` и имена фактов остаются строками: новые значения не меняют контракт. NaN при переходе границы компонента может потерять payload. Ядро и так отвергает неконечные значения, поэтому сравнение на границе считает все NaN равными.

Тест: `wit-parser` разбирает файл. Биндинги хоста и гостя генерируются из этого же файла, ручных копий типов нет. В L5/L6 проверяется отклонение чужой версии интерфейса с идентичными сигнатурами.

### L3. touchbar-logic-types

- `LogicLimits`: мягкий и жёсткий предел вызова, память экземпляра, размер стека wasm, размер обновления, запросы, подписки, текст поля, записи, сохраняемое состояние, записи журнала за вызов (64) и их длина (1 КиБ). Значения по умолчанию — из таблицы 05-logic.md.
- `CoreConfig::default()` в ядре берёт значения из `LogicLimits::default()`.
- `wire_size(&LogicUpdate)`: оценка размера, одинаковая для хоста и SDK. Байты строк плюс фиксированная цена каждого элемента, без сериализации.
- `check_budget(&LogicUpdate, &LogicLimits)`: проверка общего размера. Поэлементные проверки остаются в ядре.

Тесты: граничные значения размера, монотонность оценки (добавление элемента не уменьшает размер), совпадение значений по умолчанию с `CoreConfig`.

### L4. touchbar-logic-sdk

```rust
pub trait Logic: Sized {
    fn describe() -> LogicDescription;
    fn init(config: &str, persisted: StatePatch, ctx: &CallContext) -> Result<(Self, LogicUpdate), String>;
    fn handle(&mut self, event: LogicEvent, ctx: &CallContext) -> Result<LogicUpdate, String>;
}

touchbar_logic_sdk::export!(Controls);
```

- **`export!`** на `wasm32` генерирует экспорт через `wit-bindgen` и хранит экземпляр `Self` между вызовами. Один wasm-экземпляр обслуживает один экземпляр набора, поэтому глобального состояния хватает. На хосте макрос ничего не генерирует, и логика проверяется обычными тестами.
- **Перевод WIT ↔ модель** на стороне гостя.
- **Построители:**
  - `Description::new().action(...).field(...)`;
  - `Update::new().set(...).request(...).cancel(...).subscriptions(...)`;
  - `Request::new(tag, effect).coalesce(...).committed()`.
- **Журнал:** `log::info!` и другие уровни поверх импорта `log`.
- **Отладочная проверка бюджета** через `touchbar-logic-types`: в тестах превышение видно сразу, а не только как `LogicFault` хоста.
- **Паника гостя** становится trap (`panic = abort` для wasm-профиля). `Err` из `init` и `handle` передаётся как `result::err`.

Тесты: построители, перевод туда и обратно на хосте для всех вариантов (property на стратегиях testkit).

### L5. touchbar-wasm-fixtures

- **`build.rs`** собирает под `wasm32-unknown-unknown` плагины из `plugins/` и модули из `tests/logic-fixtures/`. Сборка идёт вложенным `cargo build --release --locked` с отдельным каталогом сборки, и окружение очищается:
  - `RUSTFLAGS` и `CARGO_ENCODED_RUSTFLAGS`, например от `cargo llvm-cov`;
  - `CARGO_TARGET_DIR`;
  - переменные `CARGO_CFG_*` хоста.
- **Упаковка в компонент** через `wit-component`. Пути выдаются константами: `fixtures::CONTROLS`, `fixtures::ECHO` и так далее.
- **`rerun-if-changed`** стоит на исходниках модулей, SDK, WIT и `Cargo.lock`.
- **`TOUCHBAR_WASM_FIXTURES=<dir>`** подставляет заранее собранные компоненты. Это запасной путь для окружений без вложенной сборки, обычный `cargo test` его не требует.
- **Код сборки** лежит отдельным модулем: позже его использует `xtask` для `plugins/*/logic.wasm`. Собранные `.wasm` в репозиторий не коммитятся.

Тестовые модули, все на SDK, кроме тех, где нужно нарушить контракт:

| Модуль | Поведение |
|---|---|
| `echo` | `init` разбирает `config` как JSON `LogicUpdate` и возвращает его. `handle` кладёт JSON полученных события и `call-context` в поле `last` |
| `trap` | паникует на заданном действии |
| `hang` | бесконечный цикл на заданном действии |
| `oom` | растит память до отказа |
| `err` | `Err` из `init` или `handle` по конфигурации |
| `budget` | обновление больше 256 КиБ |
| `log` | 1000 записей журнала за вызов |
| `persist` | поля с `persist = true`, изменение по действию |
| `version` | компонент с экспортом `wstouchbar:logic/api@9.0.0`; типы и сигнатуры идентичны v1, отличается только версия пакета |
| `wasi` | компонент с импортом WASI |

### L6. Runtime: загрузка, экземпляры, вызовы

```rust
impl Runtime {
    pub fn new(config: RuntimeConfig, store: Box<dyn PersistStore>) -> Result<Self, RuntimeError>;
    pub fn load_module(&mut self, plugin: PluginId, revision: &str, bytes: &[u8]) -> Result<(), LoadError>;
    pub fn create(&mut self, now: Timestamp, instance: InstanceId, plugin: &PluginId,
                  set: &SetRef, persist_key: &PersistKey) -> Input;      // LogicDescribed | InstanceFault
    pub fn call(&mut self, now: Timestamp, instance: InstanceId, call: CallId,
                invocation: &LogicCall) -> CallResponse;                  // Input + журнал + длительность
    pub fn dispose(&mut self, instance: InstanceId);
    pub fn persist(&mut self, key: &PersistKey, fields: &StatePatch) -> Result<(), StoreError>;
}
```

- **Движок:**
  - `wasm_component_model`;
  - `epoch_interruption`;
  - `cranelift_nan_canonicalization`;
  - `relaxed_simd_deterministic`;
  - без потоков;
  - `max_wasm_stack` из лимитов;
  - небольшой `memory_reservation`, чтобы много экземпляров не занимали адресное пространство;
  - кеш по уточнению 5.
- **Модули.**
  - **Компиляция и проверка.** `load_module` компилирует модуль, требует точное имя экспорта `wstouchbar:logic/api@1.0.0`, сверяет типы интерфейса с биндингами v1 и проверяет импорты (уточнение 4). Модуль хранится по `(plugin, revision)`.
  - **Новая ревизия.** Существующие экземпляры держат прежний компонент до уничтожения.
  - **`describe`** вызывается один раз на модуль, результат кешируется.
- **Экземпляры.**
  - **Store.** У каждого `InstanceId` свой `Store` и лимит памяти через `StoreLimits`.
  - **`create`** создаёт экземпляр, берёт описание, читает сохранённые поля из `PersistStore` и возвращает `LogicDescribed`. Любая ошибка до этого — `InstanceFault`.
  - **Повторные вызовы.** `dispose` и вызов неизвестного или уничтоженного экземпляра безопасны: ничего не делают и пишут журнал.
- **Вызовы.**
  - **Перевод.** `call` переводит `LogicCall` в `init` или `handle`, а ответ — в `LogicUpdate`, `LogicError` или `LogicFault` с исходным `CallId`.
  - **Журнал гостя** возвращается данными в `CallResponse`, отправку в journald делает оболочка на этапе 3.
- **Поток.** Вызовы синхронные, в потоке вызывающего (05-logic.md). Единственный дополнительный поток — тикер epoch.

Тесты:
- property `echo`: случайные `LogicUpdate` и `LogicEvent` из стратегий testkit проходят хост → гость → хост без изменений, NaN как в L2. 2000 случаев, в длинном прогоне 10⁵;
- детерминизм: одна последовательность событий на двух экземплярах и двух `Runtime` даёт одинаковые обновления;
- `version` даёт `InstanceFault`, хотя его сигнатуры идентичны v1: отвергается именно имя версии `wstouchbar:logic/api@9.0.0`; `wasi` даёт `InstanceFault` из-за запрещённого импорта;
- повторный `load_module` той же ревизии не перекомпилирует, а новая не трогает живые экземпляры;
- `dispose` во время жизни других экземпляров их не затрагивает.

### L7. Runtime: лимиты и сбои

- **Жёсткий предел.** Тикер увеличивает epoch раз в 1 мс, а `set_epoch_deadline(hard_ms)` с trap даёт `LogicFault{reason: timeout}`.
- **Мягкий предел** измеряется по `Instant`: превышение — предупреждение в журнале ответа, вызов не прерывается.
- **Память.** Отказ `memory.grow` по `StoreLimits` и последующий trap дают `LogicFault{reason: memory}`.
- **Стек.** Переполнение стека wasm — trap и `LogicFault`.
- **Бюджет обновления** — `check_budget` после перевода. Превышение — `LogicFault{reason: budget}`.
- **Журнал гостя.** Сверх лимита записи отбрасываются с одной записью-счётчиком, длинные сообщения обрезаются.
- **Состояние после сбоя.** Экземпляр после trap помечается мёртвым, `Store` освобождается. Ядро само выдаёт `DisposeInstance`, а после backoff — `CreateInstance` нового поколения.

Тесты используют `RuntimeConfig` с узкими пределами, чтобы не зависеть от скорости машины. Пример: проверка мягкого предела — 1 мс мягкий и 10 с жёсткий. Набор тестов:
- `trap`, `hang`, `oom`, `budget`, `err` дают нужный вид ответа;
- соседние экземпляры того же и другого модуля продолжают отвечать;
- `log` обрезается по лимиту.

### L8. Runtime: сохраняемое состояние

- **Хранилище.** Трейт `PersistStore { load(key) -> StatePatch, write(key, fields) }`, реализации `FsStore` и `MemoryStore`.
- **`FsStore`.**
  - **Пути** по D54 и D63: `$XDG_STATE_HOME/touchbar/<plugin>/<surface>/<slot>/<set>.json` и `…/layers/<set>.json`.
  - **Формат** — `{"version": 1, "fields": {...}}` с кодировкой чисел D57.
  - **Запись атомарная:** временный файл, `fsync`, `rename`.
  - **`Output::Persist`** содержит только изменённые поля (05-logic.md), поэтому `write` объединяет их с файлом.
  - **Повреждённый или чужой версии файл** не ломает загрузку: он пишется в журнал и считается пустым, при следующей записи заменяется.
  - **Лимит.** Больше 64 КиБ не записывается, ошибка возвращается вызывающему.

Тесты:
- объединение частичных записей;
- повреждённый файл;
- пересоздание экземпляра читает записанное, в том числе в новом `Runtime` (D55);
- ключи профиля и слоя не пересекаются;
- модуль `persist` через `Harness`: изменение поля → `Persist` через 1 с → новое поколение получает значение в `init`.

### L9. touchbar-sets

Перенос `touchbar-set-config` исходного проекта: 1241 строка, `schema.rs`, `panels.rs` и `state.rs`.

- **Манифест `plugin.toml`** ([04-plugins.md](../architecture/04-plugins.md)): `id`, `version`, `api`, `name`, `description`, `logic`, `requires`, `[[sets]]`. Поддерживается только `api = "1"`.
- **Схема набора.**
  - **Корень:** `version = 1`, `id`, `label`, `logic_config`, `persist`, `[units]`, `[layout]`, `[[panels]]`.
  - **Узлы:** `row`, `column`, `stack`, `responsive`, `repeat`, `button`, `text`, `icon`, `image`, `slider`, `meter`, `progress`, `scrubber`, `canvas`, `shader-effect`, `fixed-space`, `flexible-space`, `key`, `key-row`.
  - **Привязки к состоянию** `*_from` и обработчики `on_*`.
  - **Размеры** — `Length` (`u`, `%w`, `%h`).
  - **Числовые `widget_id` исходного проекта** заменяются строковыми `id`.
- **Строгость.** `deny_unknown_fields`, проверки A7 и A11, `hold_ms` при `on_long_pressed` и `press_and_hold_panel`, уникальность `id`, ссылки на панели. Ошибки — с файлом, путём в TOML и строкой.
- **`SetFile::to_spec(plugin) -> SetSpec`.** Ядру нужна только часть набора, поэтому атрибуты раскладки в `SetSpec` не попадают. Их будет читать рендер этапа 3 из того же `SetFile`.
- **Без раскладки и рендера:** построение дерева UI — этап 3.

Тесты:
- на каждое правило проверки отдельный случай;
- разбор всех наборов `plugins/*`;
- `fkeys` из 04-plugins.md с закомментированной строкой Esc и с раскомментированной (D9);
- разбор туда и обратно для `SetFile`.

### L10. testkit

- **`LogicHost`** — общий интерфейс для `Harness` поверх `FakeLogic` и нового `WasmLogic`. `WasmLogic` держит `Runtime` с `MemoryStore`, направляет в него плагины, зарегистрированные как wasm, а остальные — в `FakeLogic`. `Output::Persist` уходит в то же хранилище.
- **Сбой и время.** Зависание настоящего модуля возвращает `LogicFault` через реальный жёсткий предел. `Harness` доставляет его в текущий момент виртуального времени.
- **`plugin_dir(path)`** читает `plugin.toml` и наборы через `touchbar-sets` и берёт компонент из `touchbar-wasm-fixtures`. Ревизия — хеш содержимого, как у будущего загрузчика (D44).
- **`ModuleTester`** — модульные тесты логики без ядра:
  ```rust
  let mut m = ModuleTester::load(fixtures::CONTROLS);
  m.init("volume", vec![]).expect_subscriptions(|s| s.len() == 2);
  m.handle(audio_data("sink", 0.4, false)).expect_state("volume_text", "40%");
  m.handle(action("volume-level", ValueChanged, 0.7)).expect_request(|r| r.coalesce.as_deref() == Some("volume"));
  ```
- **DSL сценариев:** `drag(widget, from, to, duration, steps)` для ползунков и `subscription_data(set, tag, data)` для подачи данных по тегу подписки экземпляра.
- **Property I1–I12 с настоящей логикой:** стратегии этапа 1 на каталоге `controls` с `WasmLogic`. 200 случаев в CI, 10⁴ в `proptest-long`.

### L11. controls: наборы и логика

Наборы переносятся из исходного проекта (`plugins/controls/sets`, коммит `d2f9dfd`). Пиксели пересчитываются в `u`: у Touch Bar высотой 60 px 1u = 0,6 px. `battery` и `power-profile` не переносятся: по [15-roadmap.md](../architecture/15-roadmap.md) это задачи после v1.

| Набор | Раскладка | Панель |
|---|---|---|
| `volume` | `responsive`: узкий — кнопка со значком и текстом, `hold_ms = 420`, `expanded_panel` и `press_and_hold_panel`; широкий — значок, ползунок, текст | `volume-controls`: ползунок громкости и кнопка микрофона, `dismiss_on_selection` |
| `microphone` | кнопка: значок, текст, `selected_from = "microphone_muted"` | — |
| `brightness` | как `volume`, без отключения звука | `brightness-controls`: ползунок |

Один модуль обслуживает три конфигурации. `describe` объявляет объединение действий и полей, а неизвестная конфигурация — `Err` из `init`.

| Конфигурация | Подписки | Действия логики |
|---|---|---|
| `volume` | `audio.watch(default-sink)`, `audio.watch(default-source)` для кнопки микрофона в панели | ползунок: `audio.set-volume`, `coalesce = "volume"`; движение при mute сначала даёт `audio.set-mute(off)`. Кнопка микрофона: `audio.set-mute(default-source, toggle)` |
| `microphone` | `audio.watch(default-source)` | `audio.set-mute(default-source, toggle)` |
| `brightness` | `desktop.fact("display.brightness")` | `desktop.brightness.set`, `coalesce = "brightness"` |

Поведение переносится из `plugins/controls/src/logic.rs` исходного проекта без чтения через `wpctl`, очередей команд и наблюдателей — их заменили подписки и политики ядра:
- **Ползунок.** Пока ползунок удерживается, данные подписки не двигают его. Значение показывается оптимистично, после отпускания — снова значение системы. Отмена (`cancelled`) сразу возвращает последнее значение системы.
- **Микрофон** переключается оптимистично. Результат `Mute(bool)` подтверждает значение, ошибка возвращает последнее значение подписки.
- **Недоступность.** `available = false` и текст `–` при `available = false` в данных, `subscription-lost`, `Fact(None)` и результате `unsupported`.
- **Громкость выше 100 %** показывается текстом, но ползунок упирается в 1.0.
- **Поля состояния:**
  - громкость: `available`, `volume`, `volume_text`, `volume_label`, `volume_icon`, `volume_selected`;
  - микрофон: `microphone_available`, `microphone_muted`, `microphone_icon`, `microphone_text`;
  - яркость: `brightness`, `brightness_text`, `brightness_label`.
- **Значки.** Имена значков пока строки исходного проекта, соответствие символам задаёт рендер этапа 3.
- **Сохраняемых полей нет.**

### L12. controls: тесты

Модульные тесты (`ModuleTester`) — по одному на правило L11. Сценарии с ядром — именованные:

| Сценарий | Проверяет |
|---|---|
| `controls_describe_matches_sets` | все три набора из TOML проходят сверку описания, ни одного `Faulted` |
| `volume_follows_external_changes` | данные `audio.watch` меняют значение, текст и значок; во время перетаскивания ползунок стоит под пальцем |
| `volume_drag_coalesces` | 50 `value-changed` за 1 с при исполнителе 1000 мс дают два `set-volume`, последний — с конечным значением (как `slider_coalesces_writes` этапа 1, но с настоящей логикой) |
| `volume_drag_unmutes` | при mute первое движение даёт один `set-mute(off)`, затем `set-volume` |
| `volume_cancel_restores` | Fn во время перетаскивания отменяет касание (`Layer`), ползунок возвращается к значению системы |
| `volume_panel_tap_and_hold` | касание открывает `persistent`-панель, удержание 420 мс — `transient`, которая закрывается при отпускании |
| `mic_toggle_in_panel_survives_close` | A2 с настоящей логикой: переключение микрофона в панели закрывает её, а `set-mute` всё равно выполняется |
| `mic_toggle_failure_reverts` | ошибка `set-mute` возвращает значение подписки |
| `hidden_controls_hold_no_watchers` | A4: экземпляр, который не показывался, не держит подписок |
| `audio_unavailable_disables` | `subscription-lost` и `available = false` выключают элементы, восстановление их включает |
| `brightness_follows_fact` | `display.brightness` из подписки на факт двигает ползунок, ползунок даёт `desktop.brightness.set` |
| `brightness_unsupported` | `unsupported` от бэкенда выключает элемент без сбоя |
| `controls_on_two_surfaces` | два экземпляра одного набора на двух поверхностях с независимым состоянием (D35) |
| `controls_fault_is_isolated` | `LogicFault` экземпляра `volume` (через `trap`-модуль с теми же наборами) не трогает `brightness` |

Каждый сценарий проверяет инварианты на каждом входе и JSON replay итоговой трассы, как на этапе 1. Replay идёт без wasm: ответы логики записаны в трассе входами.

### L13. Приёмка

- `cargo test --workspace --locked` на свежем клоне в чистом контейнере Ubuntu 26.04 (`podman run`, только rustup). Wasm-фикстуры собираются сами.
- `fmt`, `clippy -D warnings`.
- `PROPTEST_CASES=100000` для свойств этапа 1 и `echo` — без нарушений; 10⁴ для I1–I12 с `controls`.
- Все сценарии L12 и тесты сбоев L7.
- Замеры, в протокол, без жёстких порогов:
  - компиляция `controls` без кеша и с кешем;
  - создание экземпляра;
  - медиана и p99 `handle` для `controls`;
  - размер компонента.
- Документы обновлены:
  - [05-logic.md](../architecture/05-logic.md): окончательный WIT и таблица соответствия;
  - [04-plugins.md](../architecture/04-plugins.md): формат наборов;
  - [13-testing.md](../architecture/13-testing.md): фикстуры;
  - [testing.md](../testing.md);
  - журнал решений;
  - приёмка `docs/validation/stage-2.md`.

## Вне этапа

- **Этап 3:**
  - загрузчик каталогов плагинов: пути поиска, inotify, конфликты профилей, `config.toml`, вычисление ревизий;
  - настоящие исполнители: `touchbar-audio` (PipeWire), `spawn`, `dbus`, `timer`;
  - построение дерева UI из набора и рендер;
  - бинарник сессии и отправка журнала гостя в journald;
  - `xtask` для `plugins/*/logic.wasm`.
- **Этап 5:** `display.brightness` и `desktop.brightness.set` в бэкенде GNOME. До этого яркость проверяется только с поддельным исполнителем.
- **Этап 6:** сборка wasm в пакетах.

## Риски

| Риск | Что делаем |
|---|---|
| Вложенный `cargo` в `build.rs` медленный и чувствителен к окружению (`RUSTFLAGS` от покрытия, блокировка каталога сборки) | Отдельный каталог сборки, очистка окружения, `rerun-if-changed`, запасной `TOUCHBAR_WASM_FIXTURES`. Проверка в чистом контейнере и под `cargo llvm-cov` |
| API wasmtime меняется между версиями | Версия фиксируется. Весь код wasmtime — только в `touchbar-logic-runtime` |
| Совпадение типов не подтверждает версию; развитие WIT требует отдельных биндингов | Уточнение 4: точное имя версионированного экспорта, биндинги на каждую поддерживаемую версию интерфейса, общий перевод в модель. Тест `version` с идентичными сигнатурами |
| Тесты пределов времени нестабильны на загруженной машине | Узкие `RuntimeConfig` в тестах. `hang` — бесконечный цикл, который превышает любой предел |
| Перевод WIT ↔ модель расходится на редких вариантах | Property `echo` в обе стороны на стратегиях testkit, которые покрывают все варианты |
| Формат наборов в этапе 2 тянет за собой вопросы рендера | Схема переносится как есть из исходного проекта, в `SetSpec` попадает только нужное ядру. Рендер может уточнить атрибуты раскладки без изменения `SetSpec` |
