fn main() {
    let cli = match touchbar_session::cli::Cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}\n{}", touchbar_session::cli::USAGE);
            std::process::exit(2)
        }
    };
    if cli.help {
        println!("{}", touchbar_session::cli::USAGE);
        return;
    }
    if let Err(error) = touchbar_session::driver::run(cli) {
        eprintln!("touchbar-session: {error:#}");
        std::process::exit(1);
    }
}
