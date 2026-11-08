//! Native image actions on the viewing phone, independent of the connected desktop.
#[cfg(target_os = "android")]
use serde_json::json;
use serde_json::Value;
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("phone-images")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            _app.manage(_api.register_android_plugin("dev.oah.mobile", "ImagePlugin")?);
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn image_action(app: tauri::AppHandle, action: String, name: String, mime: String, data: String) -> Result<Value, String> {
    if !matches!(action.as_str(), "save" | "copy") || !mime.starts_with("image/") {
        return Err("Unsupported image action".into());
    }
    #[cfg(target_os = "android")]
    {
        let handle = app.state::<tauri::plugin::PluginHandle<tauri::Wry>>().inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            handle.run_mobile_plugin::<Value>(&action, json!({ "name": name, "mime": mime, "data": data })).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, name, data);
        Err("Image actions require Android".into())
    }
}
