# Проверки ядра

`touchbar-testkit` запускает настоящее ядро с виртуальным временем. Поддельные логика, исполнитель и рендер не запускают процессов, не обращаются к устройствам и не читают системные часы.

```rust
use touchbar_testkit::*;
use touchbar_model::*;

let harness = scenario()
    .logic(fixtures::controls_logic())
    .catalog(fixtures::controls_with_panel())
    .fact("app.id", "org.mozilla.firefox")
    .tap("microphone")
    .expect_effect(|effect| matches!(effect, Effect::AudioSetMute { .. }))
    .run();

let json = harness.dump().to_json().unwrap();
let replay = ReplayDump::from_json(&json).unwrap().replay().unwrap();
assert_eq!(replay.core.snapshot(), harness.snapshot());
```

Для удержания используется `touch_down(1, at("microphone")).after_ms(100)`. Виджеты выбираются по актуальной карте попаданий своей поверхности. Закрытие transient-панели при отпускании якоря проверяется отдельно от выбора другим пальцем.

`Harness::send` доставляет вход и все немедленные ответы. `advance_ms` доставляет запланированные события по времени, затем по порядку постановки. `SetTimer`/`ClearTimer` управляют ближайшим таймером ядра; старые задания не срабатывают повторно. `trace` хранит входы и их выходы, включая ответы поддельных исполнителей.

`FakeLogic::register` принимает ссылку набора, необязательное описание и Rust-замыкание. Оно получает экземпляр и вызов, возвращая обновление, обычную ошибку, сбой, отложенное обновление или зависание. Без собственного описания схема выводится из виджетов фикстуры. Зависание и слишком поздний ответ дают `LogicFault` по `logic_hard_ms`; уничтоженное поколение больше не получает поддельных ответов. Логику с эффектами фикстура подключает явно через `.logic(...)`.

`FakeEffects` завершает запрос через `delay_ms` с заданным `result`; `with_handler` задаёт задержку и результат по конкретному эффекту. `timer.after` использует собственную длительность. Отмена убирает будущее завершение. Интервальные подписки выдают `Tick`; прочие данные доставляются через `subscription_after`. Отписка и потеря источника прекращают его будущие сообщения.

`FakeRenderer` располагает виджеты слева направо и возвращает `Layout`, затем `FrameDone`. Ширина задаётся через `renderer.width`, иначе используется ширина виджета или 80 пикселей. Рендер поддерживает слои, панели и геометрию scrubber; сложная настоящая раскладка остаётся в этапе рендера.

`catalog().plugin(...).set(...).profile(...).build()` строит проверяемый каталог в коде. Для нестандартных случаев доступны `SetBuilder::spec` и `ProfileBuilder::spec`.

`Recorder` снимает начальную контрольную точку и новую каждые 2000 входов. Хранятся две последние, дамп содержит от старшей точки до 4000 входов с ожидаемыми выходами. `ReplayDump` проверяет версии до разбора содержимого и сообщает номер первого расхождения. Replay работает непосредственно с ядром и записанными входами, без замыканий и других поддельных исполнителей. JSON использует точное восстановление `f64`, чтобы сериализация не меняла результат.

Дамп версии 3 сохраняет отклонённые нечисловые входы: конечные `f32`/`f64` остаются JSON-числами, NaN и бесконечности кодируются строками `f32:7fc00001` или `f64:7ff8000000000001` с точными IEEE-битами. Ширина, знак и payload NaN сохраняются. Некорректные метки отвергаются. Декодирование не отменяет проверок ядра: такие значения снова отклоняются при replay. Типы длины с собственной проверяемой грамматикой сохраняют её; недопустимая длина даёт ошибку сериализации.

Обычные проверки:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

`Harness::enable_invariant_checks()` подключает `InvariantChecker` до загрузки каталога.
Он проверяет каждый вход, включая внутренние ответы и таймеры, по выходам и
`CoreSnapshot`; первое нарушение сохраняется в `invariant_error` с номером входа
(с нуля). `finish_invariant_checks()` дополнительно требует завершить захваты
и отпустить клавиши. Для живой сессии с удерживаемыми контактами эту финальную
проверку не вызывают до завершения сценария.

Для I1/I2 используются `CaptureFinished` и исходный CaptureSnapshot; для I9 —
физические пары pressed/released и список владельцев `key_leases`. Desktop-маршрут
объединяет одинаковые клавиши между поверхностями; surface-маршрут учитывает
поверхность. Cleanup отмены/hidden проверяются с учётом порядка команд до Dispose;
обычные действия скрытым экземплярам запрещены. В I3 другой ответ ровно в момент
deadline может предшествовать Timer: после Timer или позже deadline Pending уже
быть не должно. I12 учитывает явные межповерхностные цели привязок.

`strategies::scenario_strategy()` генерирует 1–3 профиля, 1–2 поверхности,
корректные аппаратные фазы для 1–3 контактов, панели, Fn, каталоги, факты,
режимы, отзыв, ответы эффектов, данные подписок, сбои и зависания логики.
Shrinking удаляет операции целиком и уменьшает их аргументы. Контрпримеры
сохраняются в `crates/touchbar-testkit/tests/regressions/` как JSON сценария
и дамп replay; при панике fake сохраняется сам сценарий. Обычный тест
`stored_json_scenarios_replay_and_preserve_invariants` запускает эти файлы.

По умолчанию свойство использует 2000 случаев; `PROPTEST_CASES=100000` включает
длинный прогон. Вывод содержит доли сценариев, действительно наблюдавших
панель, Pending и Faulted в снимках. Для каждого состояния требуется минимум 5%.
`TOUCHBAR_REGRESSION_DIR` позволяет сохранять найденные случаи в другом каталоге.

Именованные регрессии этапа 1 находятся в
[scenarios.rs](../crates/touchbar-testkit/tests/scenarios.rs): все 19 сценариев T14
из плана, включая committed после закрытия панели/скрытия/уничтожения,
coalesce ползунка с backend 1000 и 100 мс, Fn, Pending, resume, плотность,
независимость поверхностей и desktop-маршрут клавиш. Каждый сценарий с Harness
проверяет инварианты на каждом входе и JSON replay итоговой трассы.

Длинный прогон выполняется с оптимизацией, чтобы 100000 последовательностей
укладывались в обычное время CI:

```sh
PROPTEST_CASES=100000 cargo test -p touchbar-testkit --test properties --release --locked -- --nocapture
```

Критерии и результаты этапа 1 — [приёмка](validation/stage-1.md).

После итогового ревью добавлены прямые Core-регрессии
[review_regressions.rs](../crates/touchbar-core/tests/review_regressions.rs):
Fact точно на deadline Pending до Timer освобождает захват/клавишу и
отбрасывает поздний физический callback. Сценарий скрытия проверяет порядок
Pressed → Cancelled → Hidden при занятом слоте вызова. Checker допускает
отложенный Cancelled на Hidden только если тот уже был в очереди до входа;
новый Action на Hidden остаётся нарушением I5. Для cleanup перед Dispose
учитываются очередь предыдущего снимка и порядок команд: CallLogic до Dispose
допустим, после Dispose — нарушение I8.

Тесты панели проверяют отменённые очередные и выполняющиеся вызовы,
сохранение запрета через checkpoint и отсутствие влияния на другое касание.
Persistent после обычного Up остаётся разрешённым. Harness-сценарий cleanup
дополнительно проходит проверку инвариантов и JSON replay.

## Логика и наборы этапа 2

`ModuleTester` запускает настоящий wasm без ядра:

```rust
use touchbar_testkit::*;
use touchbar_model::*;

let mut module = ModuleTester::load(touchbar_wasm_fixtures::CONTROLS);
module.init("volume", vec![])
    .handle(audio_data("sink", 1.25, false))
    .expect_state("volume", 1.0)
    .expect_state("volume_text", "125%");
module.handle(action("volume-level", ActionKind::ValueChanged, 0.7))
    .expect_request(|r| r.coalesce.as_deref() == Some("volume"));
```

`state` содержит объединённые поля, `update` — последний частичный ответ. Каждый init создаёт новый гостевой экземпляр, без старых глобальных переменных. `advance_ms` меняет время вызова. Ответы ошибок доступны в `input`.

`plugin_dir(path)` читает manifest и наборы, вычисляет revision по их содержимому и компоненту. Если logic.wasm отсутствует, тестовый loader использует соответствующую компонентную фикстуру. `WasmLogic::register_plugin` кеширует модуль и сверяет все привязки с describe; каталог активируется отдельным `Input::Catalog`. В каталог теста добавляются профили и bindings. Незарегистрированные плагины обслуживает `WasmLogic::fallback`.

DSL принимает `.logic(WasmLogic)`, `.drag(widget, from, to, duration_ms, steps)` и `.subscription_data(set, tag, data)`. Для двух поверхностей используется subscription_data_on. Данные доставляются всем активным экземплярам указанного набора и tag на выбранной поверхности.

Все 14 сценариев L12 находятся в [controls_scenarios.rs](../crates/touchbar-testkit/tests/controls_scenarios.rs); правила логики — в [controls_module.rs](../crates/touchbar-testkit/tests/controls_module.rs). Каждый сценарий проверяет I1–I12 на каждом входе, завершает захваты и сравнивает JSON replay.

```sh
cargo test -p touchbar-testkit --test controls_module --test controls_sets --test controls_scenarios --locked
PROPTEST_CONTROLS_CASES=10000 cargo test -p touchbar-testkit --test controls_properties --release --locked -- --nocapture
PROPTEST_CASES=100000 cargo test --workspace --release --locked -- --nocapture
cargo run -p touchbar-logic-runtime --example controls_benchmark --release --locked
```

controls_properties использует те же GeneratedScenario/GeneratedStep, что этап 1, с настоящим модулем и поддельными эффектами. Runtime переиспользуется между случаями для кеша компиляции; все гостевые экземпляры уничтожаются при удалении поверхностей. Тест отдельно требует успешной инициализации controls, проверяет долю Panel/Pending/Faulted и JSON replay. Смена ревизии на несовместимый hang и действия через совместимый trap проверяют fault paths.

В длинном CI обе переменные заданы явно: PROPTEST_CASES=100000, PROPTEST_CONTROLS_CASES=10000. Они независимы, поэтому можно увеличить число быстрых свойств без случайного увеличения дорогого controls-прогона. Замеры и результаты — [приёмка этапа 2](validation/stage-2.md).
