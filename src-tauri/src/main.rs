// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same binary is the terminal relay (`relay.rs`), started by the app
    // in its own session, and runs the screen search model in a process of
    // its own (`screen_search.rs`). Either way it is nothing else then: no
    // window, no Dock icon.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--relay") {
        std::process::exit(parallelist_lib::relay::main(&args[2..]));
    }
    if let Some(code) = parallelist_lib::screen_search::serve_from_args() {
        std::process::exit(code);
    }
    parallelist_lib::run()
}
