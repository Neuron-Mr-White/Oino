package dev.unipi.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * In-app updates (window.UniPiUpdate). The page reads the feed
 * (https://oino.dev/app/android/latest.json) and compares builds; this side
 * reports the installed build, downloads the APK (allow-listed host,
 * SHA-256 checked) and hands it to the system installer. Events go back
 * through window.__unipiUpdateEvent({ kind, ... }).
 */
class UpdateBridge(private val activity: Activity, private val view: WebView) {
  private val allowedHosts = setOf("oino.dev")
  @Volatile private var busy = false
  @Volatile private var ready: File? = null

  private fun emit(event: JSONObject) {
    val js = "window.__unipiUpdateEvent && window.__unipiUpdateEvent(${event})"
    view.post { view.evaluateJavascript(js, null) }
  }

  @JavascriptInterface
  fun info(): String {
    val pm = activity.packageManager
    val pkg = pm.getPackageInfo(activity.packageName, 0)
    val code = if (Build.VERSION.SDK_INT >= 28) pkg.longVersionCode else @Suppress("DEPRECATION") pkg.versionCode.toLong()
    return JSONObject()
      .put("versionName", pkg.versionName ?: "")
      .put("versionCode", code)
      .put("canInstall", if (Build.VERSION.SDK_INT >= 26) pm.canRequestPackageInstalls() else true)
      .put("busy", busy)
      .toString()
  }

  /** Downloads `url` and checks it against `sha256` (hex). Progress + result arrive as events. */
  @JavascriptInterface
  fun download(url: String, sha256: String, build: Long) {
    if (busy) return
    val parsed = try { URL(url) } catch (e: Exception) { null }
    if (parsed == null || parsed.protocol != "https" || parsed.host !in allowedHosts) {
      emit(JSONObject().put("kind", "error").put("message", "Update URL is not allowed"))
      return
    }
    busy = true
    Thread {
      val dir = File(activity.cacheDir, "updates").apply { mkdirs() }
      dir.listFiles()?.forEach { it.delete() }
      val out = File(dir, "unipi-$build.apk")
      try {
        val conn = (parsed.openConnection() as HttpURLConnection).apply {
          connectTimeout = 15000
          readTimeout = 30000
        }
        conn.connect()
        if (conn.responseCode != 200) throw IllegalStateException("Download failed (HTTP ${conn.responseCode})")
        val total = conn.contentLengthLong
        val digest = MessageDigest.getInstance("SHA-256")
        var done = 0L
        var lastEmit = 0L
        conn.inputStream.use { input ->
          out.outputStream().use { output ->
            val buf = ByteArray(64 * 1024)
            while (true) {
              val n = input.read(buf)
              if (n < 0) break
              output.write(buf, 0, n)
              digest.update(buf, 0, n)
              done += n
              val now = System.currentTimeMillis()
              if (now - lastEmit > 200) {
                lastEmit = now
                emit(JSONObject().put("kind", "progress").put("done", done).put("total", total))
              }
            }
          }
        }
        val hex = digest.digest().joinToString("") { "%02x".format(it) }
        if (!hex.equals(sha256, ignoreCase = true)) {
          out.delete()
          throw IllegalStateException("The download was damaged (checksum mismatch)")
        }
        ready = out
        emit(JSONObject().put("kind", "ready").put("build", build))
      } catch (e: Exception) {
        out.delete()
        emit(JSONObject().put("kind", "error").put("message", e.message ?: e.toString()))
      } finally {
        busy = false
      }
    }.start()
  }

  /** Opens the system installer for the downloaded APK (or the "install unknown apps" switch first). */
  @JavascriptInterface
  fun install(): String {
    val file = ready ?: return "missing"
    if (Build.VERSION.SDK_INT >= 26 && !activity.packageManager.canRequestPackageInstalls()) {
      openInstallSettings()
      return "permission"
    }
    activity.runOnUiThread {
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, "application/vnd.android.package-archive")
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
      activity.startActivity(intent)
    }
    return "started"
  }

  /**
   * UNI-214: install an APK pushed from the PC (`unipi-host push`). The core
   * already downloaded it into `<cache>/pushes/<id>/` and checked its
   * SHA-256; only files under that directory are accepted. Returns
   * "started" | "permission" (opened the "install unknown apps" switch) |
   * "missing".
   */
  @JavascriptInterface
  fun installFile(path: String): String {
    val file = pushedFile(activity, path) ?: return "missing"
    ready = file
    return install()
  }

  @JavascriptInterface
  fun openInstallSettings() {
    if (Build.VERSION.SDK_INT < 26) return
    activity.runOnUiThread {
      activity.startActivity(
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}"))
          .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
      )
    }
  }
}

/** A file the core fetched for a push (`<cache>/pushes/…`), or null. */
internal fun pushedFile(activity: Activity, path: String): File? {
  val root = File(activity.cacheDir, "pushes").canonicalFile
  val file = try { File(path).canonicalFile } catch (e: Exception) { return null }
  return if (file.isFile && file.path.startsWith(root.path + File.separator)) file else null
}
