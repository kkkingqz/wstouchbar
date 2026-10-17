use std::{
    io::{self, Write},
    process::ExitCode,
};
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "Usage: touchbard probe | --hand-back\n\nprobe        Root-only Touch Bar display diagnostic; restores native mode afterwards.\n--hand-back  Root-only crash cleanup: return USB to native and restore owned autodim."
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
        eprintln!("Usage: touchbard probe | --hand-back (see --help)");
        return ExitCode::from(2);
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
