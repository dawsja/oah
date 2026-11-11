//! Keep the phone's system bars in step with the app's selected CSS theme: Android's bar icons.
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
pub async fn phone_theme(app: tauri::AppHandle, dark: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        app.state::<Appearance<tauri::Wry>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>("apply", serde_json::json!({ "dark": dark }))
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, dark);
    Ok(())
}
