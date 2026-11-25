// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The same binary runs the screen search model in a process of its own
    // (`screen_search.rs`), and is nothing else then: no window, no Dock icon.
    if let Some(code) = parallelist_lib::screen_search::serve_from_args() {
        std::process::exit(code);
    }
    parallelist_lib::run()
}
