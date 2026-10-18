use std::{
    io::{self, Write},
    process::ExitCode,
};
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "Usage: touchbard [--config DIR] [--runtime DIR] | probe | --hand-back\n\n--config DIR   Administrator configuration directory (default /etc/touchbar).\n--runtime DIR  Daemon runtime directory (default /run/touchbar).\nprobe        Root-only Touch Bar display diagnostic; restores native mode afterwards.\n--hand-back  Root-only crash cleanup: return USB to native and restore owned autodim."
        );
        return ExitCode::SUCCESS;
    }
    if args.as_slice() == ["--hand-back"] {
        if !rustix::process::geteuid().is_root() {
            eprintln!("touchbard --hand-back requires root");
            return ExitCode::FAILURE;
        }
        return match futures_lite::future::block_on(touchbard::backlight::hand_back(
            touchbard::device::Roots::default(),
        )) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("touchbard --hand-back: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args.as_slice() != ["probe"] {
        let mut config = std::path::PathBuf::from("/etc/touchbar");
        let mut roots = touchbard::device::Roots::default();
        let mut seen = std::collections::BTreeSet::new();
        let mut i = 0;
        while i < args.len() {
            if !matches!(args[i].as_str(), "--config" | "--runtime")
                || i + 1 == args.len()
                || !seen.insert(args[i].clone())
                || !std::path::Path::new(&args[i + 1]).is_absolute()
            {
                eprintln!(
                    "Usage: touchbard [--config DIR] [--runtime DIR] | probe | --hand-back (see --help)"
                );
                return ExitCode::from(2);
            }
            if args[i] == "--config" {
                config = (&args[i + 1]).into();
            } else {
                roots.run = (&args[i + 1]).into();
            }
            i += 2;
        }
        if !rustix::process::geteuid().is_root() {
            eprintln!("touchbard requires root");
            return ExitCode::FAILURE;
        }
        let result = (|| -> io::Result<()> {
            let mut drivers = touchbard::runtime::linux::Linux::new(roots.clone(), config)?;
            let filter = touchbard::runtime::logging::init(&drivers.config().log)?;
            drivers.set_log_filter(filter);
            let mut runtime = touchbard::runtime::Runtime::new(
                drivers.config().clone(),
                drivers.recovery(),
                &roots.run,
                drivers,
            )?;
            futures_lite::future::block_on(runtime.run())
        })();
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("touchbard: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let result = futures_lite::future::block_on(touchbard::probe::run()).and_then(|report| {
        let mut out = io::stdout().lock();
        serde_json::to_writer_pretty(&mut out, &report)?;
        writeln!(out)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("touchbard probe: {e}");
            ExitCode::FAILURE
        }
    }
}
