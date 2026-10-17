use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use touchbar_effects::dbus::*;
use touchbar_model::*;
struct PrivateBus {
    child: Child,
    address: String,
}
impl PrivateBus {
    fn new() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
}
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
// Generated server introspection is test-only, outside deterministic core/replay state.
#[allow(clippy::disallowed_types)]
mod service {
    pub(super) struct Service {
        pub(super) value: i32,
    }
    #[zbus::interface(name = "org.example.Touchbar.Test")]
    impl Service {
        fn echo(&self, text: &str) -> String {
            text.into()
        }
        #[zbus(property)]
        fn value(&self) -> i32 {
            self.value
        }
        #[zbus(property)]
        fn set_value(&mut self, value: i32) {
            self.value = value;
        }
    }
    pub(super) struct SlowService {
        pub(super) entered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    #[zbus::interface(name = "org.example.Touchbar.Slow")]
    impl SlowService {
        #[zbus(property)]
        async fn value(&self) -> i32 {
            self.entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            async_io::Timer::after(std::time::Duration::from_millis(500)).await;
            1
        }
    }
}
use service::Service;
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    futures_lite::future::race(future, async {
        async_io::Timer::after(Duration::from_secs(3)).await;
        panic!("private D-Bus operation timed out")
    })
    .await
}
fn call(method: &str, signature: &str, json: &str) -> DbusCall {
    DbusCall {
        bus: Bus::Session,
        destination: "org.example.Touchbar.Test".into(),
        path: "/test".into(),
        interface: "org.example.Touchbar.Test".into(),
        method: method.into(),
        args: DbusValue {
            signature: signature.into(),
            json: json.into(),
        },
    }
}
#[test]
#[ignore = "requires a private dbus-daemon"]
fn env_dbus_calls_signals_properties_and_owner_loss() {
    futures_lite::future::block_on(async {
        let bus = PrivateBus::new();
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.example.Touchbar.Test")
            .unwrap()
            .serve_at("/test", Service { value: 3 })
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = Client::with_addresses(Some(bus.address.clone()), Some(bus.address.clone()));
        let reply = bounded(client.call(&call("Echo", "s", "[\"hello\"]")))
            .await
            .unwrap();
        assert_eq!(
            reply,
            DbusValue {
                signature: "s".into(),
                json: "[\"hello\"]".into()
            }
        );
        let mut signals = client
            .signals(DbusMatch {
                bus: Bus::Session,
                sender: Some("org.example.Touchbar.Test".into()),
                path: Some("/test".into()),
                interface: Some("org.example.Touchbar.Test".into()),
                member: Some("Changed".into()),
            })
            .await
            .unwrap();
        server
            .emit_signal(
                None::<&str>,
                "/test",
                "org.example.Touchbar.Test",
                "Changed",
                &(42u32,),
            )
            .await
            .unwrap();
        let signal = bounded(signals.next()).await.unwrap();
        assert_eq!(signal.signature, "u");
        assert_eq!(signal.json, "[42]");
        let mut property = client
            .property(DbusProperty {
                bus: Bus::Session,
                destination: "org.example.Touchbar.Test".into(),
                path: "/test".into(),
                interface: "org.example.Touchbar.Test".into(),
                property: "Value".into(),
            })
            .await
            .unwrap();
        assert_eq!(bounded(property.next()).await.unwrap().json, "[3]");
        let mut set = call(
            "Set",
            "ssv",
            r#"["org.example.Touchbar.Test","Value",{"sig":"i","value":7}]"#,
        );
        set.interface = "org.freedesktop.DBus.Properties".into();
        bounded(client.call(&set)).await.unwrap();
        assert_eq!(bounded(property.next()).await.unwrap().json, "[7]");
        // D-Bus serials are chosen by the peer and need not be monotonic.
        let values = std::collections::BTreeMap::from([(
            "Value".to_string(),
            zbus::zvariant::OwnedValue::from(9i32),
        )]);
        let message = zbus::Message::signal(
            "/test",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .serial(std::num::NonZeroU32::new(1).unwrap())
        .build(&("org.example.Touchbar.Test", values, Vec::<String>::new()))
        .unwrap();
        server.send(&message).await.unwrap();
        assert_eq!(bounded(property.next()).await.unwrap().json, "[9]");

        let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        server
            .object_server()
            .at(
                "/slow",
                service::SlowService {
                    entered: entered.clone(),
                },
            )
            .await
            .unwrap();
        server
            .request_name("org.example.Touchbar.Slow")
            .await
            .unwrap();
        let pending = client.property(DbusProperty {
            bus: Bus::Session,
            destination: "org.example.Touchbar.Slow".into(),
            path: "/slow".into(),
            interface: "org.example.Touchbar.Slow".into(),
            property: "Value".into(),
        });
        let release = async {
            while !entered.load(std::sync::atomic::Ordering::SeqCst) {
                async_io::Timer::after(Duration::from_millis(1)).await;
            }
            server
                .release_name("org.example.Touchbar.Slow")
                .await
                .unwrap();
        };
        let (result, ()) = bounded(futures_lite::future::zip(pending, release)).await;
        assert!(
            matches!(result, Err(EffectError::Unavailable)),
            "snapshot from departed owner was accepted"
        );
        server.close().await.unwrap();
        assert!(bounded(signals.next()).await.is_err());
        assert!(bounded(property.next()).await.is_err());
    })
}
