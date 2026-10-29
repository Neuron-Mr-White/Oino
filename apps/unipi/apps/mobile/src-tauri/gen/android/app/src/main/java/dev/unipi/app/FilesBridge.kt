package dev.unipi.app

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.webkit.JavascriptInterface
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.util.Base64

/**
 * Getting host files onto/into other apps (window.UniPiFiles, UNI-169):
 *  - saveDownload(name, mime, base64): into the public Downloads
 *    collection under Download/unipi via MediaStore (API 29+, no
 *    storage permission); below 29 a legacy best-effort write.
 *  - openWith(name, mime, base64): cache file + FileProvider
 *    content:// URI + ACTION_VIEW system chooser ("Open with…").
 * Both take the file inline (base64) and synchronously return a JSON
 * string {ok: true, uri?} / {ok: false, error}.
 */
class FilesBridge(private val activity: Activity) {

  private fun fail(error: String): String =
    JSONObject().put("ok", false).put("error", error).toString()

  private fun decode(base64: String): ByteArray? = try {
    Base64.getDecoder().decode(base64)
  } catch (_: IllegalArgumentException) {
    null
  }

  @JavascriptInterface
  fun saveDownload(name: String, mime: String, base64: String): String {
    val bytes = decode(base64) ?: return fail("bad base64")
    return try {
      val uri = saveToDownloads(name, mime.ifBlank { "application/octet-stream" }) { it.write(bytes) }
      JSONObject().put("ok", true).put("uri", uri.toString()).toString()
    } catch (e: SecurityException) {
      fail("no permission to write Downloads: ${e.message}")
    } catch (e: Exception) {
      fail("${e.javaClass.simpleName}: ${e.message}")
    }
  }

  @JavascriptInterface
  fun openWith(name: String, mime: String, base64: String): String {
    val bytes = decode(base64) ?: return fail("bad base64")
    val safeName = name.substringAfterLast('/').ifBlank { "file" }
    val type = mime.ifBlank { "application/octet-stream" }
    val dir = File(activity.cacheDir, "shared").apply { mkdirs() }
    // One file per open; stale copies go with the cache.
    val file = File(dir, System.currentTimeMillis().toString() + "_" + safeName)
    return try {
      file.writeBytes(bytes)
      val uri: Uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
      val view = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, type)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      val chooser = Intent.createChooser(view, "Open $safeName with").apply {
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.startActivity(chooser)
      JSONObject().put("ok", true).toString()
    } catch (e: ActivityNotFoundException) {
      file.delete()
      fail("no app can open $safeName ($type)")
    } catch (e: Exception) {
      file.delete()
      fail("${e.javaClass.simpleName}: ${e.message}")
    }
  }

  /** UNI-224: open a web link in the phone's browser. `window.open` can't: with no
   * multi-window support the WebView loaded it over the app, with no way back. */
  @JavascriptInterface
  fun openUrl(url: String): String {
    val uri = Uri.parse(url)
    if (uri.scheme !in setOf("http", "https", "mailto")) return fail("not a web link")
    return try {
      activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      JSONObject().put("ok", true).toString()
    } catch (e: ActivityNotFoundException) {
      fail("no app can open $url")
    } catch (e: Exception) {
      fail("${e.javaClass.simpleName}: ${e.message}")
    }
  }

  /** UNI-214: save a pushed file (already fetched + verified into `<cache>/pushes/`) to Downloads/unipi. */
  @JavascriptInterface
  fun saveFile(path: String, name: String, mime: String): String {
    val file = pushedFile(activity, path) ?: return fail("not a pushed file")
    return try {
      // Streamed (UNI-225): Files → Download of a big file lands here too, never read it into memory.
      val uri = saveToDownloads(name, mime.ifBlank { "application/octet-stream" }) { out -> file.inputStream().use { it.copyTo(out, 256 * 1024) } }
      JSONObject().put("ok", true).put("uri", uri.toString()).toString()
    } catch (e: SecurityException) {
      fail("no permission to write Downloads: ${e.message}")
    } catch (e: Exception) {
      fail("${e.javaClass.simpleName}: ${e.message}")
    }
  }

  /** UNI-214: "Open with…" for a pushed file, straight from the cache (no copy). */
  @JavascriptInterface
  fun openFile(path: String, name: String, mime: String): String {
    val file = pushedFile(activity, path) ?: return fail("not a pushed file")
    val safeName = name.substringAfterLast('/').ifBlank { file.name }
    val type = mime.ifBlank { "application/octet-stream" }
    return try {
      val uri: Uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
      val view = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, type)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      val chooser = Intent.createChooser(view, "Open $safeName with").apply {
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.startActivity(chooser)
      JSONObject().put("ok", true).toString()
    } catch (e: ActivityNotFoundException) {
      fail("no app can open $safeName ($type)")
    } catch (e: Exception) {
      fail("${e.javaClass.simpleName}: ${e.message}")
    }
  }

  private fun saveToDownloads(name: String, mime: String, write: (java.io.OutputStream) -> Unit): Uri {
    val safeName = name.substringAfterLast('/').ifBlank { "file" }
    if (Build.VERSION.SDK_INT >= 29) {
      val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, safeName)
        put(MediaStore.Downloads.MIME_TYPE, mime)
        put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/unipi")
        put(MediaStore.Downloads.IS_PENDING, 1)
      }
      val resolver = activity.contentResolver
      val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        ?: throw IllegalStateException("MediaStore insert returned null")
      resolver.openOutputStream(uri)?.use { write(it) }
        ?: throw IllegalStateException("no stream for $uri")
      values.clear()
      values.put(MediaStore.Downloads.IS_PENDING, 0)
      resolver.update(uri, values, null, null)
      return uri
    }
    // Legacy (< 29): direct write under the public Downloads dir; needs
    // WRITE_EXTERNAL_STORAGE, so the SecurityException path surfaces a
    // clear error instead of a crash.
    val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "unipi")
    dir.mkdirs()
    val file = File(dir, safeName)
    file.outputStream().use { write(it) }
    activity.sendBroadcast(
      Intent(Intent.ACTION_MEDIA_SCANNER_SCAN_FILE, Uri.fromFile(file)),
    )
    return Uri.fromFile(file)
  }
}
