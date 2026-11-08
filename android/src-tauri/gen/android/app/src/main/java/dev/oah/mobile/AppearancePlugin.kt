package dev.oah.mobile

import android.app.Activity
import androidx.core.view.WindowInsetsControllerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class ThemeArgs {
    var dark: Boolean = false
}

@TauriPlugin
class AppearancePlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun apply(invoke: Invoke) {
        val args = invoke.parseArgs(ThemeArgs::class.java)
        activity.runOnUiThread {
            val controller = WindowInsetsControllerCompat(activity.window, activity.window.decorView)
            controller.isAppearanceLightStatusBars = !args.dark
            controller.isAppearanceLightNavigationBars = !args.dark
            invoke.resolve()
        }
    }
}
