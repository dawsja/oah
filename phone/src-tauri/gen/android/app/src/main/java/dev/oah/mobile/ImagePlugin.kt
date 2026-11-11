package dev.oah.mobile

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class ImageArgs {
    lateinit var name: String
    lateinit var mime: String
    lateinit var data: String
}

@TauriPlugin
class ImagePlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun save(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ImageArgs::class.java)
            val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = args.mime
                putExtra(Intent.EXTRA_TITLE, args.name)
            }
            startActivityForResult(invoke, intent, "saved")
        } catch (e: Exception) { invoke.reject(e.message ?: "Could not open save dialog") }
    }

    @ActivityCallback
    fun saved(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.resolve(JSObject().put("saved", false))
            return
        }
        try {
            val uri = result.data?.data ?: throw Exception("No destination selected")
            val args = invoke.parseArgs(ImageArgs::class.java)
            val bytes = Base64.decode(args.data, Base64.DEFAULT)
            activity.contentResolver.openOutputStream(uri, "wt")?.use { it.write(bytes) }
                ?: throw Exception("Could not open destination")
            invoke.resolve(JSObject().put("saved", true))
        } catch (e: Exception) { invoke.reject(e.message ?: "Could not save image") }
    }

    @Command
    fun copy(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(ImageArgs::class.java)
            val bytes = Base64.decode(args.data, Base64.DEFAULT)
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                ?: throw Exception("Could not decode image")
            val dir = File(activity.cacheDir, "clipboard-images").apply { mkdirs() }
            val file = File.createTempFile("image-", ".png", dir)
            try {
                file.outputStream().use {
                    if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) throw Exception("Could not encode image")
                }
            } finally { bitmap.recycle() }
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newUri(activity.contentResolver, args.name, uri))
            invoke.resolve(JSObject().put("saved", true))
        } catch (e: Exception) { invoke.reject(e.message ?: "Could not copy image") }
    }
}
