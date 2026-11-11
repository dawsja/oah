//! Keep the phone's system bars in step with the app's selected CSS theme: Android's bar icons,
//! the iPhone's window (and with it the status bar's text).
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(target_os = "android")]
struct Appearance<R: Runtime>(tauri::plugin::PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("phone-appearance")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _app.manage(Appearance(_api.register_android_plugin("dev.oah.mobile", "AppearancePlugin")?));
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn phone_theme(app: tauri::AppHandle, dark: bool, system: Option<bool>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        app.state::<Appearance<tauri::Wry>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>("apply", serde_json::json!({ "dark": dark }))
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    if cfg!(target_os = "ios") {
        // Following the phone again hands the window back to it.
        let style = if system == Some(true) {
            "system"
        } else if dark {
            "dark"
        } else {
            "light"
        };
        crate::native::<serde_json::Value>(&app, "appearance", serde_json::json!({ "style": style })).await?;
    }
    #[cfg(target_os = "android")]
    let _ = system;
    Ok(())
}
