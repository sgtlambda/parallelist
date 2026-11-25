// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same binary is the terminal relay (`relay.rs`), started by the app
    // in its own session.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--relay") {
        std::process::exit(parallelist_lib::relay::main(&args[2..]));
    }
    parallelist_lib::run()
}
