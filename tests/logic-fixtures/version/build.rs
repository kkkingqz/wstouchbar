#[path = "../contract_build.rs"]
mod contract_build;
fn main() {
    contract_build::generate();
}
