package dev.oah.mobile

import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

// Android 17 (API 37) requires a runtime grant to reach devices on the local network.
private const val ACCESS_LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The whole app is talking to a computer on the same network, so ask up front.
    if (Build.VERSION.SDK_INT >= 37 &&
      ContextCompat.checkSelfPermission(this, ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED
    ) {
      ActivityCompat.requestPermissions(this, arrayOf(ACCESS_LOCAL_NETWORK), 0)
    }
  }
}
