fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("render") {
        args.next();
        touchbarctl::render_png(&touchbarctl::RenderOptions::parse(args)?)
    } else if args
        .peek()
        .map(String::as_str)
        .is_some_and(|s| s == "--help" || s == "-h")
    {
        println!(
            "touchbarctl [--simulator] status [--json] | dump [-o FILE] | reload | sim fact NAME=VALUE | sim fact --unset NAME | sim theme light|dark\ntouchbarctl render PLUGIN/SET [--state FILE] [--surface WxH@Nppmm] [--theme dark|light] [--plugins DIR] -o FILE"
        );
        Ok(())
    } else {
        print!(
            "{}",
            touchbarctl::run_control(touchbarctl::ControlOptions::parse(args)?)?
        );
        Ok(())
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("touchbarctl: {error}");
        std::process::exit(2);
    }
}
