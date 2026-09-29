mod cache;
mod cli;
mod installer;
mod local;
mod network;
mod recipe;
mod version;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = cli::run(&args);
    std::process::exit(code);
}
