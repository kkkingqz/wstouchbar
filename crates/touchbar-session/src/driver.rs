//! Native async-io loop. All runtime, GL and subprocess creation stays on this thread.
use crate::simulator::{Simulator, SimulatorDesktop, SimulatorEvent};
use crate::{cli::Cli, surface::*, *};
use anyhow::{Result, anyhow};
use futures_lite::StreamExt;
use std::{os::fd::AsFd, path::PathBuf, rc::Rc, sync::Arc, time::Duration};
use touchbar_desktop::{DesktopBackend, FakeBackend, NullBackend};
use touchbar_loader::{DesktopChoice, LoadRequest, ReloadWatcher, UserConfig, Worker, read_config};
use tracing_subscriber::{EnvFilter, prelude::*, reload};
type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;
fn logging(config: &UserConfig) -> Result<FilterHandle> {
    let value = std::env::var("TOUCHBAR_LOG").unwrap_or_else(|_| config.log.clone());
    let filter = EnvFilter::try_new(&value).unwrap_or_else(|e| {
        eprintln!("invalid log filter {value:?}: {e}; using info");
        EnvFilter::new("info")
    });
    let (filter, handle) = reload::Layer::new(filter);
    let journal = std::env::var_os("JOURNAL_STREAM").and_then(|_| tracing_journald::layer().ok());
    let stderr = journal
        .is_none()
        .then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    tracing_subscriber::registry()
        .with(filter)
        .with(journal)
        .with(stderr)
        .try_init()
        .map_err(|e| anyhow!(e))?;
    Ok(handle)
}
fn state_home() -> Result<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".local/state"))
        })
        .ok_or_else(|| anyhow!("no absolute XDG_STATE_HOME or HOME"))
}
enum Ready {
    Executor(Event),
    Backend(touchbar_desktop::BackendEvent),
    Loaded(touchbar_loader::PreparedLoad),
    Simulator(SimulatorEvent),
    Touchbar(std::result::Result<crate::touchbard::ConnectionEvent, String>),
    Control(touchbar_control::Incoming),
    Watch,
    Wake,
    Stop,
    Failure(String),
}
/// Poll transport once between core slices, preserving incomplete clients in Server.
pub async fn poll_control(
    server: &mut touchbar_control::Server,
    session: &mut Session,
    fake: Option<&FakeBackend>,
) -> Result<bool> {
    let incoming = futures_lite::future::poll_once(server.next())
        .await
        .transpose()?
        .flatten();
    for issue in server.take_issues() {
        session.warning(&issue);
    }
    if let Some(incoming) = incoming {
        let (reply, reload) = session.control(incoming.request.command.clone(), fake);
        server.respond(incoming, reply)?;
        Ok(reload)
    } else {
        Ok(false)
    }
}
pub fn run(cli: Cli) -> Result<()> {
    futures_lite::future::block_on(run_async(cli))
}
async fn run_async(cli: Cli) -> Result<()> {
    let touchbard_path = cli.touchbard_socket.clone().or_else(|| {
        (!cli.simulator && cli.headless.is_none())
            .then(|| PathBuf::from("/run/touchbar/session.sock"))
    });

    let request = LoadRequest::standard(cli.plugins).map_err(|e| anyhow!(e))?;
    let startup = read_config(&request.config_file).unwrap_or_else(|error| {
        eprintln!("{error}; using default startup configuration");
        UserConfig::default()
    });
    let log = logging(&startup)?;
    let (desktop_tx, desktop_rx) = async_channel::bounded(512);
    let choice = cli.desktop.unwrap_or(startup.desktop);
    let fake_desktop = (choice == DesktopChoice::Fake).then(FakeBackend::default);
    let mut desktop: Box<dyn DesktopBackend> = match choice {
        DesktopChoice::Fake => Box::new(fake_desktop.clone().expect("fake backend")),
        DesktopChoice::Null => Box::<NullBackend>::default(),
        DesktopChoice::Gnome => {
            tracing::warn!("GNOME backend is not available before stage 5; using null");
            Box::<NullBackend>::default()
        }
        DesktopChoice::Auto => {
            if std::env::var("XDG_CURRENT_DESKTOP")
                .is_ok_and(|s| s.to_ascii_lowercase().contains("gnome"))
            {
                tracing::warn!("GNOME backend is not available before stage 5; using null");
            }
            Box::<NullBackend>::default()
        }
    };
    if cli.simulator {
        desktop = Box::new(SimulatorDesktop::new(desktop));
    }
    let _desktop_events_lifetime = desktop_tx.clone();
    desktop.start(desktop_tx)?;
    let desktop: Rc<dyn DesktopBackend> = Rc::from(desktop);
    let effects = if cli.fake_effects {
        touchbar_effects::Engine::fake(desktop.clone()).0
    } else {
        let audio = touchbar_audio::NativeAudio::connect(touchbar_audio::NativeOptions::default())?;
        let helper = std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow!("session binary has no parent directory"))?
            .join("touchbar-spawn-helper");
        touchbar_effects::Engine::native(
            touchbar_effects::Config::new(helper),
            desktop.clone(),
            Arc::new(audio),
        )
    };
    let home = touchbar_control::state_home(&state_home()?, cli.simulator);
    let mut control =
        touchbar_control::Server::bind(&touchbar_control::runtime_dir()?, cli.simulator)?;
    let clock = MonotonicClock::default();
    let options = SessionOptions::new(home.clone(), startup.clone());
    let store = touchbar_logic_runtime::FsStore::with_limit(home, options.limits.max_persist_bytes);
    let mut session = Session::new(options, clock.clone(), Box::new(store), desktop, effects)?;
    if let Some(info) = cli.headless {
        let (surface, _) = HeadlessSurface::new(info);
        session.add_surface(Box::new(surface))?;
    }
    let mut simulator = if cli.simulator {
        let simulator = Simulator::connect(cli.surfaces).await?;
        for surface in simulator.surfaces() {
            session.add_surface(surface)?;
        }
        Some(simulator)
    } else {
        None
    };
    let mut touchbar = touchbard_path
        .map(crate::touchbard::Connector::new)
        .transpose()?;
    let worker = Worker::new(session.compiler());
    let mut reload_pending = false;
    let mut loading = true;
    let mut loaded_once = false;
    let mut pending_load = None;
    worker.submit(request.clone()).map_err(|e| anyhow!(e))?;
    let mut watcher = ReloadWatcher::new(&request).map_err(|e| anyhow!(e))?;
    let watcher_fd = async_io::Async::new(rustix::io::dup(watcher.as_fd())?)?;
    let mut signals =
        async_signal::Signals::new([async_signal::Signal::Term, async_signal::Signal::Int])?;
    let result: Result<()> = async {
        loop {
            session.pump()?;
            if let Some(touchbar) = &touchbar {
                session.touchbard_status = Some(touchbar.status());
            }
            for issue in control.take_issues() {
                session.warning(&issue);
            }
            for issue in watcher.take_issues() {
                session.warning(&format!("configuration watcher: {issue}"));
            }
            if session.has_pending() {
                reload_pending |=
                    poll_control(&mut control, &mut session, fake_desktop.as_ref()).await?;
            }
            let deadline = session
                .next_deadline()
                .map(|t| Duration::from_millis(t.0))
                .into_iter()
                .chain(watcher.deadline())
                .min();
            let now = Duration::from_millis(clock.now().0);
            let timeout = if session.has_pending() {
                Some(Duration::ZERO)
            } else {
                deadline.map(|at| at.saturating_sub(now))
            };
            let ready = {
                let executor = async { Ready::Executor(session.next_executor().await) };
                let backend = async {
                    desktop_rx
                        .recv()
                        .await
                        .map(Ready::Backend)
                        .unwrap_or_else(|_| Ready::Failure("desktop event channel closed".into()))
                };
                let simulated = async {
                    if let Some(simulator) = &mut simulator {
                        simulator
                            .next_event()
                            .await
                            .map(Ready::Simulator)
                            .unwrap_or_else(|e| Ready::Failure(format!("Wayland simulator: {e}")))
                    } else {
                        std::future::pending::<Ready>().await
                    }
                };
                let hardware = async {
                    if let Some(touchbar) = &mut touchbar {
                        Ready::Touchbar(
                            touchbar.next_event(&clock).await.map_err(|e| e.to_string()),
                        )
                    } else {
                        std::future::pending::<Ready>().await
                    }
                };
                let controlled = async {
                    control
                        .next()
                        .await
                        .map(|incoming| incoming.map_or(Ready::Wake, Ready::Control))
                        .unwrap_or_else(|e| Ready::Failure(format!("control socket: {e}")))
                };
                let loaded = async {
                    worker
                        .results
                        .recv()
                        .await
                        .map(Ready::Loaded)
                        .unwrap_or_else(|_| Ready::Failure("loader worker stopped".into()))
                };
                // Return once polling changes the debounce deadline, so the main
                // loop rebuilds its timer. WouldBlock clears cached readiness.
                let watch = async {
                    watcher_fd
                        .read_with(|_| {
                            let before = watcher.deadline();
                            let changed = watcher
                                .poll(Duration::from_millis(clock.now().0))
                                .map_err(std::io::Error::other)?;
                            if changed || watcher.deadline() != before {
                                Ok(changed)
                            } else {
                                Err(std::io::ErrorKind::WouldBlock.into())
                            }
                        })
                        .await
                        .map(|changed| if changed { Ready::Watch } else { Ready::Wake })
                };
                let timer = async {
                    if let Some(delay) = timeout {
                        async_io::Timer::after(delay).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                    Ready::Wake
                };
                let stop = async {
                    let _ = signals.next().await;
                    Ready::Stop
                };
                use futures_lite::future::{or, race};
                // Signals and elapsed deadlines cannot be starved by an active
                // producer; the other sources share polling priority.
                or(
                    stop,
                    or(
                        timer,
                        race(
                            loaded,
                            race(
                                backend,
                                race(
                                    race(hardware, race(simulated, controlled)),
                                    race(executor, async {
                                        watch.await.unwrap_or_else(|e| {
                                            Ready::Failure(format!("configuration watcher: {e}"))
                                        })
                                    }),
                                ),
                            ),
                        ),
                    ),
                )
                .await
            };
            match ready {
                Ready::Stop => break,
                Ready::Touchbar(Ok(crate::touchbard::ConnectionEvent::Protocol(event))) => {
                    match event {
                        crate::touchbard::ClientEvent::Welcome(_) => {
                            let handle = touchbar
                                .as_ref()
                                .and_then(crate::touchbard::Connector::handle)
                                .expect("connected Welcome");
                            let added = crate::touchbard::TouchbarSurface::new(handle.clone())
                                .map_err(anyhow::Error::msg)
                                .and_then(|surface| session.add_surface(Box::new(surface)));
                            if let Err(error) = added {
                                session.warning(&format!("Touch Bar surface: {error:#}"));
                                handle.close();
                            }
                        }
                        crate::touchbard::ClientEvent::Surface(event) => {
                            session.handle_event(Event::Surface(event))?
                        }
                    }
                }
                Ready::Touchbar(Ok(crate::touchbard::ConnectionEvent::Disconnected {
                    surface,
                    error,
                })) => {
                    session.warning(&error);
                    if let Some(surface) = surface {
                        session.handle_event(Event::Input(touchbar_core::Input::Revoked {
                            surface,
                        }))?;
                        session.handle_event(Event::Input(
                            touchbar_core::Input::SurfaceRemoved(surface),
                        ))?;
                    }
                }
                Ready::Touchbar(Err(error)) => {
                    session.warning(&format!("Touch Bar socket watcher: {error}"));
                    if let Some(mut failed) = touchbar.take() {
                        if let Some(surface) = failed.handle().and_then(|h| h.info()).map(|i| i.id)
                        {
                            session.handle_event(Event::Input(touchbar_core::Input::Revoked {
                                surface,
                            }))?;
                            session.handle_event(Event::Input(
                                touchbar_core::Input::SurfaceRemoved(surface),
                            ))?;
                        }
                        let mut status = failed.status();
                        failed.close();
                        status.connected = false;
                        status.path = None;
                        status.error = Some(error);
                        session.touchbard_status = Some(status);
                    }
                }
                Ready::Failure(error) | Ready::Simulator(SimulatorEvent::Fault(error)) => {
                    return Err(anyhow!(error));
                }
                Ready::Executor(event) => session.handle_event(event)?,
                Ready::Control(incoming) => {
                    let (reply, reload) =
                        session.control(incoming.request.command.clone(), fake_desktop.as_ref());
                    reload_pending |= reload;
                    control.respond(incoming, reply)?;
                }
                Ready::Backend(mut event) => {
                    if cli.simulator
                        && let touchbar_desktop::BackendEvent::Ready(caps) = &mut event
                    {
                        caps.commands.extend([
                            touchbar_desktop::CommandKind::Key,
                            touchbar_desktop::CommandKind::Shortcut,
                        ]);
                    }
                    session.handle_event(Event::Desktop(event))?;
                }
                Ready::Simulator(SimulatorEvent::Surface(event)) => {
                    session.handle_event(Event::Surface(event))?
                }
                Ready::Simulator(SimulatorEvent::ToggleTheme) => {
                    if let Some(fake) = &fake_desktop {
                        let light = fake.fact(&"appearance.scheme".into())
                            == Some(touchbar_model::FactValue::Text("light".into()));
                        if let Err(error) = fake.set_fact(
                            "appearance.scheme".into(),
                            Some(touchbar_model::FactValue::Text(
                                if light { "dark" } else { "light" }.into(),
                            )),
                        ) {
                            session.warning(&format!("theme toggle: {error}"));
                        }
                    }
                }
                Ready::Wake => {
                    session.handle_event(Event::Wake)?;
                    if watcher
                        .poll(Duration::from_millis(clock.now().0))
                        .map_err(|e| anyhow!(e))?
                    {
                        reload_pending = true;
                    }
                }
                Ready::Watch => reload_pending = true,
                Ready::Loaded(prepared) => {
                    loading = false;
                    pending_load = Some(prepared);
                }
            }
            if !session.has_pending()
                && let Some(prepared) = pending_load.take()
            {
                let previous_catalog = session.catalog().clone();
                let previous_config = session.config().clone();
                let initial = !loaded_once;
                let report = session.load(prepared);
                loaded_once |= report.accepted;
                if initial
                    || session.catalog() != &previous_catalog
                    || session.config() != &previous_config
                    || !report.accepted
                {
                    tracing::info!(
                        event = "catalog_loaded",
                        accepted = report.accepted,
                        issues = report.issues.len()
                    );
                }
                if report.accepted
                    && std::env::var_os("TOUCHBAR_LOG").is_none()
                    && let Ok(filter) = EnvFilter::try_new(&session.config().log)
                {
                    log.reload(filter).map_err(|e| anyhow!(e))?;
                }
            }

            if simulator.as_ref().is_some_and(Simulator::closed) {
                break;
            }
            if reload_pending && !loading {
                worker.submit(request.clone()).map_err(|e| anyhow!(e))?;
                loading = true;
                reload_pending = false;
            }
        }
        Ok(())
    }
    .await;
    let shutdown = session.shutdown();
    let drained = session.finish_shutdown().await;
    if let Some(touchbar) = &mut touchbar {
        touchbar.close();
    }
    result.and(shutdown).and(drained)
}
