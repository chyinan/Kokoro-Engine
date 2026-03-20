// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// test comment
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri_appkokoro_engine_lib::run()
}
