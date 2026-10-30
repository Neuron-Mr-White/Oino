package dev.unipi.app

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/**
 * UNI-161 §2: UnifiedPush delivery while the app isn't running. The
 * distributor (the ntfy app) decrypts its own transport layer and hands
 * us the raw ntfy JSON body as [PushMessage.content]; we pull `message`
 * out of it and decrypt *that* with our own per-pairing key
 * (NotifyCrypto.kt) — the distributor never sees our plaintext either.
 *
 * `onNewEndpoint` is forwarded to JS (`__unipiPushEndpoint`) so it can
 * call `notify_register` on the paired host; `onMessage` shows a native
 * notification directly (no page may be loaded at all).
 */
class PushServiceImpl : PushService() {
  override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
    val prefs = getSharedPreferences(NotificationBridge.PREFS_NAME, Context.MODE_PRIVATE)
    prefs.edit().putString("push_endpoint_url", endpoint.url).apply()
    // The page (if loaded) picks this up and calls notify_register on the
    // host; MainActivity.onWebViewCreate also reads push_endpoint_url on
    // load to catch an endpoint that arrived while the page wasn't open.
  }

  override fun onMessage(message: PushMessage, instance: String) {
    val prefs = getSharedPreferences(NotificationBridge.PREFS_NAME, Context.MODE_PRIVATE)
    val keyB64 = prefs.getString(NotificationBridge.PREF_KEY, null) ?: return
    val body = String(message.content, Charsets.UTF_8)
    // The content IS the ntfy JSON publish body; `message` within it is
    // our `unipi1:` envelope (see docs/m6/PROTOCOL.md).
    val ntfyMessage = try {
      JSONObject(body).optString("message", body)
    } catch (e: Exception) {
      body
    }
    if (!NotifyCrypto.isEncryptedEnvelope(ntfyMessage)) return
    val payload = try {
      NotifyCrypto.decrypt(keyB64, ntfyMessage)
    } catch (e: Exception) {
      return
    }

    NotificationBridge.ensureChannel(this)
    val builder = androidx.core.app.NotificationCompat.Builder(this, NotificationBridge.CHANNEL_ID)
      .setSmallIcon(android.R.drawable.ic_dialog_info)
      .setContentTitle(payload.title)
      .setContentText(payload.body)
      .setAutoCancel(true)
      .setPriority(androidx.core.app.NotificationCompat.PRIORITY_HIGH)

    val deepLink = buildString {
      append("unipi://chat?")
      payload.host?.let { append("host=").append(android.net.Uri.encode(it)).append('&') }
      payload.pid?.let { append("pid=").append(it).append('&') }
      payload.dialogId?.let { append("dialog=").append(it) }
    }
    val openIntent = Intent(Intent.ACTION_VIEW, android.net.Uri.parse(deepLink))
      .setPackage(packageName)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val notificationId = (payload.dialogId ?: payload.pid ?: 0) + 1000
    builder.setContentIntent(
      android.app.PendingIntent.getActivity(this, notificationId, openIntent, android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE),
    )

    payload.options?.take(3)?.forEachIndexed { i, option ->
      val actionIntent = Intent(this, NotifyActionReceiver::class.java).apply {
        putExtra(NotifyActionReceiver.EXTRA_NOTIFICATION_ID, notificationId)
        putExtra(NotifyActionReceiver.EXTRA_ACTION, option)
      }
      val pending = android.app.PendingIntent.getBroadcast(
        this,
        notificationId * 10 + i,
        actionIntent,
        android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE,
      )
      builder.addAction(0, option, pending)
    }

    ContextCompat.getSystemService(this, NotificationManager::class.java)?.notify(notificationId, builder.build())
  }

  override fun onRegistrationFailed(reason: FailedReason, instance: String) {
    // Best-effort: the next app-open re-registers (UnifiedPush.register
    // is idempotent); no user-facing error needed for a background retry.
  }

  override fun onUnregistered(instance: String) {
    val prefs = getSharedPreferences(NotificationBridge.PREFS_NAME, Context.MODE_PRIVATE)
    prefs.edit().remove("push_endpoint_url").apply()
  }
}
