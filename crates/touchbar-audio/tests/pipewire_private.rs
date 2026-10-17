use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use touchbar_audio::*;
use touchbar_model::*;
struct Private {
    dir: tempfile::TempDir,
    children: Vec<Child>,
    address: String,
}
impl Private {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let mut this = Self {
            dir,
            children: vec![bus],
            address: address.trim().into(),
        };
        let config = this.dir.path().join("config/pipewire/pipewire.conf.d");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("99-null.conf"),r#"
context.objects = [
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test.sink.one node.description = "Sink one" media.class = Audio/Sink audio.position = [ FL FR ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test.sink.two node.description = "Sink two" media.class = Audio/Sink audio.position = [ FL FR ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test.source node.description = "Source" media.class = Audio/Source audio.position = [ MONO ] } }
]
"#).unwrap();
        this.start("pipewire");
        let start = Instant::now();
        while !this.dir.path().join("pipewire-0").exists() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "PipeWire did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        this.start("wireplumber");
        this
    }
    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("XDG_RUNTIME_DIR", self.dir.path())
            .env("PIPEWIRE_RUNTIME_DIR", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address);
        cmd
    }
    fn start(&mut self, program: &str) {
        let log = fs::File::create(self.dir.path().join(format!("{program}.log"))).unwrap();
        let child = self
            .command(program)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        self.children.push(child);
    }
    fn run(&self, program: &str, args: &[&str]) -> String {
        let output = self.command(program).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{program} {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn id(&self, name: &str) -> String {
        let dump = self.run("pw-dump", &[]);
        let values: serde_json::Value = serde_json::from_str(&dump).unwrap();
        values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["info"]["props"]["node.name"] == name)
            .unwrap_or_else(|| panic!("no node {name}: {dump}"))["id"]
            .to_string()
    }
}
impl Drop for Private {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "PW LOG {}",
                fs::read_to_string(self.dir.path().join("pipewire.log")).unwrap_or_default()
            );
            eprintln!(
                "WP LOG {}",
                fs::read_to_string(self.dir.path().join("wireplumber.log")).unwrap_or_default()
            );
        }
        for child in self.children.iter_mut().rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
async fn state(label: &str, watch: &AudioWatch, test: impl Fn(&AudioState) -> bool) -> AudioState {
    futures_lite::future::or(
        async {
            loop {
                let s = watch
                    .events
                    .recv()
                    .await
                    .expect("watch closed")
                    .expect("source lost");
                if test(&s) {
                    return s;
                }
            }
        },
        async {
            async_io::Timer::after(Duration::from_secs(10)).await;
            panic!("audio update timed out: {label}")
        },
    )
    .await
}
fn close(value: f64, expected: f64) -> bool {
    (value - expected).abs() < 0.01
}
#[test]
#[ignore = "requires private PipeWire and WirePlumber; cargo xtask test-env pipewire"]
fn env_pipewire_native_watch_write_external_change_and_default_switch() {
    let mut private = Private::new();
    let sink = private.id("test.sink.one");
    let other = private.id("test.sink.two");
    let source = private.id("test.source");
    let start = Instant::now();
    loop {
        let output = private
            .command("wpctl")
            .args(["set-default", &sink])
            .output()
            .unwrap();
        if output.status.success() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "WirePlumber not ready: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    private.run("wpctl", &["set-default", &source]);
    private.run(
        "pw-metadata",
        &[
            "-n",
            "default",
            "0",
            "default.audio.sink",
            r#"{"name":"test.sink.one"}"#,
            "Spa:String:JSON",
        ],
    );
    private.run(
        "pw-metadata",
        &[
            "-n",
            "default",
            "0",
            "default.audio.source",
            r#"{"name":"test.source"}"#,
            "Spa:String:JSON",
        ],
    );
    let backend = NativeAudio::connect(NativeOptions {
        remote: Some(
            private
                .dir
                .path()
                .join("pipewire-0")
                .to_str()
                .unwrap()
                .into(),
        ),
    })
    .unwrap();
    let sink_watch = backend.watch(AudioTarget::DefaultSink).unwrap();
    let source_watch = backend.watch(AudioTarget::DefaultSource).unwrap();
    futures_lite::future::block_on(async {
        assert_eq!(
            state("initial sink", &sink_watch, |s| s.available)
                .await
                .name,
            "Sink one"
        );
        assert_eq!(
            state("initial source", &source_watch, |s| s.available)
                .await
                .name,
            "Source"
        );
        // An independent client changes linear SPA amplitude through wpctl's cubic scale.
        private.run("wpctl", &["set-volume", &sink, "0.5"]);
        assert!(close(
            state("external volume", &sink_watch, |s| close(s.volume, 0.5))
                .await
                .volume,
            0.5
        ));
        assert_eq!(
            backend
                .command(AudioCommand::SetVolume {
                    target: AudioTarget::DefaultSink,
                    level: 0.7
                })
                .await,
            Ok(EffectResult::None)
        );
        state("set volume", &sink_watch, |s| close(s.volume, 0.7)).await;
        assert!(
            private
                .run("wpctl", &["get-volume", &sink])
                .contains("0.70")
        );
        assert_eq!(
            backend
                .command(AudioCommand::SetMute {
                    target: AudioTarget::DefaultSink,
                    value: MuteValue::On
                })
                .await,
            Ok(EffectResult::Mute(true))
        );
        state("mute", &sink_watch, |s| s.muted).await;
        assert_eq!(
            backend
                .command(AudioCommand::SetMute {
                    target: AudioTarget::DefaultSink,
                    value: MuteValue::Toggle
                })
                .await,
            Ok(EffectResult::Mute(false))
        );
        state("toggle", &sink_watch, |s| !s.muted).await;
        private.run("wpctl", &["set-default", &other]);
        private.run(
            "pw-metadata",
            &[
                "-n",
                "default",
                "0",
                "default.audio.sink",
                r#"{"name":"test.sink.two"}"#,
                "Spa:String:JSON",
            ],
        );
        state("switch default", &sink_watch, |s| {
            s.available && s.name == "Sink two"
        })
        .await;
        backend
            .command(AudioCommand::SetVolume {
                target: AudioTarget::DefaultSink,
                level: 0.3,
            })
            .await
            .unwrap();
        state("new default volume", &sink_watch, |s| close(s.volume, 0.3)).await;
        assert!(
            private
                .run("wpctl", &["get-volume", &other])
                .contains("0.30")
        );
        assert!(
            private
                .run("wpctl", &["get-volume", &sink])
                .contains("0.70")
        );
    });
    for child in private.children.iter_mut().skip(1).rev() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    private.children.truncate(1);
    futures_lite::future::block_on(futures_lite::future::or(
        async {
            loop {
                if let Err(error) = sink_watch.events.recv().await.expect("lost event") {
                    assert_eq!(error, EffectError::Unavailable);
                    break;
                }
            }
        },
        async {
            async_io::Timer::after(Duration::from_secs(5)).await;
            panic!("missing SubscriptionLost")
        },
    ));
    let recovered = backend.watch(AudioTarget::DefaultSink).unwrap();
    assert!(!recovered.events.recv_blocking().unwrap().unwrap().available);
    // A deterministic replacement server isolates client recovery from the
    // session manager's startup policy.
    fs::write(private.dir.path().join("config/pipewire/pipewire.conf.d/98-metadata.conf"),r#"
context.objects = [ { factory = metadata args = {
 metadata.name = default
 metadata.values = [ { key = default.audio.sink type = "Spa:String:JSON" value = { name = test.sink.one } } ]
} } ]
"#).unwrap();
    private.start("pipewire");
    let ready = Instant::now();
    loop {
        let out = private.command("pw-dump").output().unwrap();
        if out.status.success() {
            break;
        }
        assert!(ready.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    private.run(
        "pw-metadata",
        &[
            "-n",
            "default",
            "0",
            "default.audio.sink",
            r#"{"name":"test.sink.one"}"#,
            "Spa:String:JSON",
        ],
    );

    futures_lite::future::block_on(state("reconnect", &recovered, |s| {
        s.available && s.name == "Sink one"
    }));
    // Dropping the private services never touches the user's PipeWire daemon.
}
