use std::{
    fs,
    time::{Duration, Instant},
};
use touchbard::{
    Command,
    device::Roots,
    runtime::{Drivers, linux::Linux, marker::Recovery},
};
#[test]
fn system_writes_survive_mode_generation_and_restore_autodim_in_order() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: dir.path().join("sys"),
            dev: dir.path().join("dev"),
            run: dir.path().join("run"),
        };
        let params = roots.sys.join("module/hid_appletb_kbd/parameters");
        fs::create_dir_all(&params).unwrap();
        fs::create_dir_all(&roots.dev).unwrap();
        for (name, value) in [
            ("autodim", "Y\n"),
            ("dim_timeout", "30\n"),
            ("idle_timeout", "15\n"),
        ] {
            fs::write(params.join(name), value).unwrap();
        }
        let mut linux = Linux::new(roots.clone(), dir.path().join("admin")).unwrap();
        linux.dispatch(Command::OwnAutodim);
        linux.dispatch(Command::WriteRecovery(true));
        linux.dispatch(Command::Device(
            touchbard::machine::mode::Command::ReleaseHardware { generation: 1 },
        ));
        linux.dispatch(Command::RestoreAutodim);
        linux.dispatch(Command::Device(
            touchbard::machine::mode::Command::ReleaseHardware { generation: 2 },
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            futures_lite::future::or(linux.next(), async {
                async_io::Timer::at(deadline).await;
                panic!("system jobs failed to drain")
            })
            .await
            .unwrap();
            if linux.idle() {
                break;
            }
        }
        assert!(Recovery::new(&roots.run).read().unwrap());
        assert_eq!(fs::read_to_string(params.join("autodim")).unwrap(), "Y\n");
        assert!(!roots.run.join("autodim").exists());
    });
}

#[test]
fn restart_with_owned_autodim_marker_reports_ownership_even_if_parameter_is_disabled() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: dir.path().join("sys"),
            dev: dir.path().join("dev"),
            run: dir.path().join("run"),
        };
        let params = roots.sys.join("module/hid_appletb_kbd/parameters");
        fs::create_dir_all(&params).unwrap();
        fs::create_dir_all(&roots.dev).unwrap();
        fs::write(params.join("autodim"), "Y\n").unwrap();
        touchbard::backlight::Autodim::new(&roots)
            .take_over()
            .unwrap();
        let mut linux = Linux::new(roots.clone(), dir.path().join("admin")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let events = futures_lite::future::or(linux.next(), async {
                async_io::Timer::at(deadline).await;
                panic!("no owned autodim fact after restart")
            })
            .await
            .unwrap();
            if events.contains(&touchbard::Event::Autodim(true)) {
                break;
            }
        }
        linux.dispatch(Command::RestoreAutodim);
        while !linux.idle() {
            linux.next().await.unwrap();
        }
        assert_eq!(fs::read_to_string(params.join("autodim")).unwrap(), "Y\n");
    });
}

#[test]
fn shutdown_reports_failed_autodim_restore_and_keeps_the_recovery_marker() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: dir.path().join("sys"),
            dev: dir.path().join("dev"),
            run: dir.path().join("run"),
        };
        let params = roots.sys.join("module/hid_appletb_kbd/parameters");
        fs::create_dir_all(&params).unwrap();
        fs::create_dir_all(&roots.dev).unwrap();
        fs::write(params.join("autodim"), "Y\n").unwrap();
        touchbard::backlight::Autodim::new(&roots)
            .take_over()
            .unwrap();
        let mut linux = Linux::new(roots.clone(), dir.path().join("admin")).unwrap();
        fs::remove_file(params.join("autodim")).unwrap();
        fs::create_dir(params.join("autodim")).unwrap();
        linux.dispatch(Command::RestoreAutodim);
        assert!(
            linux
                .finish(Instant::now() + Duration::from_secs(2))
                .await
                .is_err()
        );
        assert!(roots.run.join("autodim").exists());
    });
}
