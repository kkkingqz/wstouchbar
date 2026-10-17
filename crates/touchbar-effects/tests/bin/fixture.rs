use std::{
    io::{Read, Write},
    time::Duration,
};
// The escaped-exit fixture deliberately leaves collection to the supervisor.
#[allow(clippy::zombie_processes)]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("echo") => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            print!(
                "{}",
                serde_json::json!({"arg":args[2],"env":std::env::var("TOUCHBAR_TEST_VALUE").unwrap(),"stdin":input})
            );
            eprint!("error stream");
            std::process::exit(7);
        }
        Some("flood") => {
            for _ in 0..128 {
                std::io::stdout().write_all(&[b'a'; 8192]).unwrap();
                std::io::stderr().write_all(&[b'b'; 8192]).unwrap();
            }
        }
        Some("many-lines") => {
            for _ in 0..1000 {
                println!("line");
            }
        }
        Some("lines") => print!("one\ntwo\r\ntail"),
        Some("descendants") | Some("escaped") | Some("escaped-exit") => {
            let mut command = std::process::Command::new(&args[0]);
            if args[1] != "descendants" {
                detach(&mut command);
            }
            let mut child = command
                .arg("ignore-term")
                .arg(format!("{}.ready", args[2]))
                .spawn()
                .unwrap();
            while !std::path::Path::new(&format!("{}.ready", args[2])).exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::fs::write(&args[2], format!("{} {}", std::process::id(), child.id())).unwrap();
            if args[1] != "escaped-exit" {
                let _ = child.wait();
            }
        }
        Some("ignore-term") => {
            ignore_term();
            std::fs::write(&args[2], "ready").unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        Some("spawn-parent") => {
            use touchbar_model::*;
            let pool = touchbar_effects::spawn::SpawnPool::new(args[2].clone().into(), 4, 64);
            let _process = pool
                .start(
                    InstanceId {
                        number: 1,
                        generation: Generation(1),
                    },
                    SpawnSpec::new(vec![
                        args[0].clone(),
                        args.get(4).cloned().unwrap_or_else(|| "descendants".into()),
                        args[3].clone(),
                    ]),
                    false,
                    None,
                )
                .unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        Some("idle") => {
            println!("{}", std::process::id());
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        Some("sleep") => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        _ => std::process::exit(2),
    }
}
#[allow(unsafe_code)]
fn ignore_term() {
    // SAFETY: fixture-only process installs an ignore disposition to exercise SIGKILL escalation.
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
}

#[allow(unsafe_code)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe, called only in the forked fixture child.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}
