// No console window on Windows in release builds (the desktop build is only for trying it out).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oah_phone_lib::run()
}
