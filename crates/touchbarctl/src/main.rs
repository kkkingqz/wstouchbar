fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref(){
        Some("render")=>touchbarctl::render_png(&touchbarctl::RenderOptions::parse(args)?),
        _=>Err("usage: touchbarctl render PLUGIN/SET [--state FILE] [--surface WxH@Nppmm] [--theme dark|light] [--plugins DIR] -o FILE".into()),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("touchbarctl: {error}");
        std::process::exit(2);
    }
}
