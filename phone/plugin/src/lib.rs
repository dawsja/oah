//! What the phone app needs from iOS itself (`ios/Sources`): the look of the status bar, the
//! share sheet and clipboard for images, a tap of haptics, Bonjour for nearby desktops, and
//! keeping a folder out of backups. On Android the app's Kotlin plugins do this work
//! (`src-tauri/gen/android`), and everywhere else there is nothing to reach.

use serde::{de::DeserializeOwned, Serialize};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_oah_phone);

/// The Swift side, in the app's state after [`init`].
pub struct Native<R: Runtime> {
    #[cfg(target_os = "ios")]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(target_os = "ios"))]
    _runtime: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Native<R> {
    /// Runs one of the Swift plugin's commands and waits for its answer. It blocks, and Swift
    /// often needs the main thread to answer: call it from a blocking task, never from `setup`.
    pub fn run<T: DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T, String> {
        #[cfg(target_os = "ios")]
        {
            self.handle.run_mobile_plugin(command, payload).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = (command, payload);
            Err("Only on an iPhone.".into())
        }
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("oah-phone")
        .setup(|app, _api| {
            #[cfg(target_os = "ios")]
            app.manage(Native { handle: _api.register_ios_plugin(init_plugin_oah_phone)? });
            #[cfg(not(target_os = "ios"))]
            app.manage(Native::<R> { _runtime: std::marker::PhantomData });
            Ok(())
        })
        .build()
}
