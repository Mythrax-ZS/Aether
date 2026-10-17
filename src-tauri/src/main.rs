// Release builds run as a GUI app (no console window).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    aether_app::run();
}
