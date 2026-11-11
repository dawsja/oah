// The app's own commands call the Swift side through `Native::run`; nothing here is a command
// the webview can call.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).ios_path("ios").build();
}
