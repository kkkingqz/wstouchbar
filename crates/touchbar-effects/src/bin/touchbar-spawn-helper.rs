fn main() {
    if let Err(error) = touchbar_effects::spawn::helper_main() {
        eprintln!("touchbar-spawn-helper: {error}");
        std::process::exit(125);
    }
}
