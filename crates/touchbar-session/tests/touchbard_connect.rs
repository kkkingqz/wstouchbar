use std::time::Duration;
use touchbar_protocol::{self as wire, Mode, ModeReason};
use touchbar_session::{
    ManualClock,
    touchbard::{ClientEvent, ConnectionEvent, Connector},
};
use touchbar_testkit::{FakeTouchbarDisplay, FakeTouchbard};
fn display() -> FakeTouchbarDisplay {
    FakeTouchbarDisplay {
        header: wire::SwapchainHeader {
            logical_width: 16,
            logical_height: 4,
            physical_width: 16,
            physical_height: 4,
            format: wire::XRGB8888,
            pitch: 64,
            buffer_size: 256,
            buffer_count: 3,
            frame_damage: true,
        },
        width_mm100: 800,
        height_mm100: 200,
    }
}
#[test]
fn waits_for_missing_socket_and_parent_directory_without_polling() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent/session.sock");
        let mut connector = Connector::new(path.clone()).unwrap();
        let clock = ManualClock::default();
        assert!(
            futures_lite::future::poll_once(connector.next_event(&clock))
                .await
                .is_none()
        );
        std::fs::write(dir.path().join("recently-used.xbel"), b"irrelevant").unwrap();
        assert!(
            futures_lite::future::poll_once(connector.next_event(&clock))
                .await
                .is_none()
        );
        let peer = async {
            async_io::Timer::after(Duration::from_millis(10)).await;
            std::fs::create_dir(path.parent().unwrap()).unwrap();
            let mut fake = FakeTouchbard::bind(&path, display()).unwrap();
            fake.next_event().await.unwrap();
            fake.next_event().await.unwrap();
            fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
            fake
        };
        let (event, _fake) = futures_lite::future::zip(connector.next_event(&clock), peer).await;
        assert!(matches!(
            event.unwrap(),
            ConnectionEvent::Protocol(ClientEvent::Welcome(_))
        ));
        assert!(connector.status().connected);
    });
}
#[test]
fn rejected_connections_back_off_and_reconnect_uses_a_fresh_surface_id() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.sock");
        let mut fake = FakeTouchbard::bind(&path, display()).unwrap();
        let clock = ManualClock::default();
        let mut c = Connector::new(path).unwrap();
        for (n, delay) in [100u64, 200, 400].into_iter().enumerate() {
            assert!(
                futures_lite::future::poll_once(c.next_event(&clock))
                    .await
                    .is_none()
            );
            assert!(c.status().connected);
            fake.next_event().await.unwrap();
            fake.next_event().await.unwrap();
            assert!(
                futures_lite::future::poll_once(c.next_event(&clock))
                    .await
                    .is_none()
            );
            fake.disconnect();
            let e = c.next_event(&clock).await.unwrap();
            assert!(matches!(
                e,
                ConnectionEvent::Disconnected { surface: None, .. }
            ));
            clock.advance(delay - 1);
            assert!(
                futures_lite::future::poll_once(c.next_event(&clock))
                    .await
                    .is_none()
            );
            assert!(
                !c.status().connected,
                "retry {n} was too early; repeated Hello must not reset backoff"
            );
            clock.advance(1);
        }
        let connect = async {
            loop {
                if let ConnectionEvent::Protocol(ClientEvent::Welcome(info)) =
                    c.next_event(&clock).await.unwrap()
                {
                    break info;
                }
            }
        };
        let peer = async {
            fake.next_event().await.unwrap();
            fake.next_event().await.unwrap();
            fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        };
        let (first, ()) = futures_lite::future::zip(connect, peer).await;
        while futures_lite::future::poll_once(c.next_event(&clock))
            .await
            .is_some()
        {}
        fake.disconnect();
        assert!(
            matches!(c.next_event(&clock).await.unwrap(),ConnectionEvent::Disconnected{surface:Some(id),..} if id==first.id)
        );
        clock.advance(1000);
        let connect = async {
            loop {
                if let ConnectionEvent::Protocol(ClientEvent::Welcome(info)) =
                    c.next_event(&clock).await.unwrap()
                {
                    break info;
                }
            }
        };
        let peer = async {
            fake.next_event().await.unwrap();
            fake.next_event().await.unwrap();
        };
        let (second, ()) = futures_lite::future::zip(connect, peer).await;
        assert_ne!(first.id, second.id);
    });
}
#[test]
fn socket_appearance_is_not_lost_behind_unrelated_inotify_batches() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.sock");
        let clock = ManualClock::default();
        let mut c = Connector::new(path.clone()).unwrap();
        assert!(
            futures_lite::future::poll_once(c.next_event(&clock))
                .await
                .is_none()
        );
        for n in 0..4000 {
            std::fs::write(dir.path().join(format!("unrelated-{n}")), b"").unwrap();
        }
        let mut fake = FakeTouchbard::bind(&path, display()).unwrap();
        let exchange = async {
            let peer = async {
                fake.next_event().await.unwrap();
                fake.next_event().await.unwrap();
                fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
            };
            futures_lite::future::zip(c.next_event(&clock), peer)
                .await
                .0
                .unwrap()
        };
        let outcome = futures_lite::future::or(async { Some(exchange.await) }, async {
            async_io::Timer::after(Duration::from_secs(2)).await;
            None
        })
        .await;
        assert!(matches!(
            outcome,
            Some(ConnectionEvent::Protocol(ClientEvent::Welcome(_)))
        ));
    });
}
