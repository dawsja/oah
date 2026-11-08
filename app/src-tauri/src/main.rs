// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `oah mcp`: an MCP client's stdio bridge to the running app (see Settings → MCP).
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        std::process::exit(oah_app_lib::mcp_bridge());
    }
    oah_app_lib::run()
}
