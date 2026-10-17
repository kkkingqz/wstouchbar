use std::{
    io::{self, Write},
    process::ExitCode,
};
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "Usage: touchbard probe\n\nprobe  Root-only Touch Bar display diagnostic; restores native mode afterwards."
        );
        return ExitCode::SUCCESS;
    }
    if args.as_slice() != ["probe"] {
        eprintln!("Usage: touchbard probe (see --help)");
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
