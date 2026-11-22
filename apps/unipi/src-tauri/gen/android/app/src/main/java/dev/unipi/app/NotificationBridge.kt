package dev.unipi.app

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.os.Build
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import org.unifiedpush.android.connector.UnifiedPush

/**
 * UNI-161 - window.UniPiNotify: the JS bridge for native notifications,
 * UnifiedPush registration and the pending-action handoff, following the
 * window.UniPiUpdate pattern (UpdateBridge.kt).
 *
 * - show/cancel: a plain local notification (e.g. the ongoing "Running"
 *   activity indicator) on the "Unipi - needs you" channel.
 * - requestPermission: POST_NOTIFICATIONS on API 33+ (manifest already
 *   declares it; this just triggers the system prompt).
 * - registerPush/pushStatus: UnifiedPush registration with whatever
 *   distributor is installed (the ntfy app, if present); the phone sends
 *   the resulting endpoint to the host via notify_register from JS.
 * - setKey: stores the per-pairing E2E key in the app's private
 *   SharedPreferences (NOT the WebView's storage) so PushServiceImpl can
 *   decrypt messages that arrive while no page is loaded.
 * - takePendingAction: on startup/resume, JS reads one queued action
 *   (an ask_user answer or permission allow/deny tapped from a
 *   notification while the app was closed) and answers the dialog once
 *   the bridge reconnects.
 */
class NotificationBridge(private val activity: Activity, private val view: WebView) {
  companion object {
    const val CHANNEL_ID = "unipi-needs-you"
    /** UNI-223: the ongoing "Running … · Stop" notification lives on its own
     *  LOW-importance channel. On the HIGH "needs you" channel every update
     *  (a new tool, "Thinking" → "Writing", the elapsed time) re-alerted with
     *  sound/heads-up — a buzz on every chat change. */
    const val RUNNING_CHANNEL_ID = "unipi-running"
    const val PREFS_NAME = "unipi_notify"
    const val PREF_KEY = "notify_key_b64"
    const val PREF_PENDING_ACTION = "pending_action_json"
    const val REQUEST_CODE_NOTIFICATIONS = 9201

    fun ensureChannel(context: Context) {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
      val manager = context.getSystemService(NotificationManager::class.java)
      if (manager.getNotificationChannel(CHANNEL_ID) == null) {
        val channel = NotificationChannel(CHANNEL_ID, "Unipi - needs you", NotificationManager.IMPORTANCE_HIGH)
        channel.description = "Pi is waiting for an answer, or a background task finished."
        manager.createNotificationChannel(channel)
      }
      if (manager.getNotificationChannel(RUNNING_CHANNEL_ID) == null) {
        val running = NotificationChannel(RUNNING_CHANNEL_ID, "Unipi - running", NotificationManager.IMPORTANCE_LOW)
        running.description = "A quiet, ongoing notice while pi works, with a Stop button."
        running.setShowBadge(false)
        manager.createNotificationChannel(running)
      }
    }
  }

  private fun prefs(): SharedPreferences = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

  /** Shows (or updates, same id) a notification. actionsJson is up to
   *  3 {label, action} objects rendered as action buttons (ask_user
   *  options / permission allow-deny / the ongoing run's Stop button). */
  @JavascriptInterface
  fun show(id: Int, title: String, body: String, ongoing: Boolean, actionsJson: String?) {
    ensureChannel(activity)
    val builder = NotificationCompat.Builder(activity, if (ongoing) RUNNING_CHANNEL_ID else CHANNEL_ID)
      .setSmallIcon(android.R.drawable.ic_dialog_info)
      .setContentTitle(title)
      .setContentText(body)
      .setOngoing(ongoing)
      .setAutoCancel(!ongoing)
      // UNI-223: an update to the same id never alerts again.
      .setOnlyAlertOnce(true)
      .setSilent(ongoing)
      .setPriority(if (ongoing) NotificationCompat.PRIORITY_LOW else NotificationCompat.PRIORITY_HIGH)

    val deepLinkIntent = Intent(Intent.ACTION_VIEW, android.net.Uri.parse("unipi://chat"))
      .setPackage(activity.packageName)
    builder.setContentIntent(
      PendingIntent.getActivity(activity, id, deepLinkIntent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE),
    )

    try {
      val actions = actionsJson?.let { JSONArray(it) }
      if (actions != null) {
        for (i in 0 until minOf(actions.length(), 3)) {
          val action = actions.getJSONObject(i)
          val label = action.optString("label", "")
          val actionId = action.optString("action", "")
          val actionIntent = Intent(activity, NotifyActionReceiver::class.java).apply {
            putExtra(NotifyActionReceiver.EXTRA_NOTIFICATION_ID, id)
            putExtra(NotifyActionReceiver.EXTRA_ACTION, actionId)
          }
          val pending = PendingIntent.getBroadcast(
            activity,
            id * 10 + i,
            actionIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
          )
          builder.addAction(0, label, pending)
        }
      }
    } catch (e: Exception) {
      // malformed actionsJson: show the notification without buttons rather than fail silently
    }

    val manager = ContextCompat.getSystemService(activity, NotificationManager::class.java)
    manager?.notify(id, builder.build())
  }

  @JavascriptInterface
  fun cancel(id: Int) {
    val manager = ContextCompat.getSystemService(activity, NotificationManager::class.java)
    manager?.cancel(id)
  }

  /** Triggers the POST_NOTIFICATIONS system prompt on API 33+; a no-op
   *  (always "granted") on older versions. Result arrives as an event:
   *  window.__unipiNotifyPermission(granted). */
  @JavascriptInterface
  fun requestPermission() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
      emit("window.__unipiNotifyPermission && window.__unipiNotifyPermission(true)")
      return
    }
    activity.runOnUiThread {
      androidx.core.app.ActivityCompat.requestPermissions(activity, arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), REQUEST_CODE_NOTIFICATIONS)
    }
  }

  /** Registers with the user's current (or default) UnifiedPush
   *  distributor. The resulting endpoint arrives at PushServiceImpl.
   *  onNewEndpoint, which forwards it to JS via __unipiPushEndpoint. */
  @JavascriptInterface
  fun registerPush() {
    activity.runOnUiThread {
      UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { success ->
        if (success) {
          UnifiedPush.register(activity, messageForDistributor = "Unipi - needs you")
        } else {
          emit("window.__unipiPushStatus && window.__unipiPushStatus(${JSONObject().put("state", "no_distributor")})")
        }
      }
    }
  }

  /** {state: "registered"|"no_distributor"|"not_registered", distributor?: string} */
  @JavascriptInterface
  fun pushStatus(): String {
    val distributors = UnifiedPush.getDistributors(activity)
    val ack = try { UnifiedPush.getAckDistributor(activity) } catch (e: Exception) { null }
    val result = JSONObject()
    return when {
      ack != null -> result.put("state", "registered").put("distributor", ack).toString()
      distributors.isEmpty() -> result.put("state", "no_distributor").toString()
      else -> result.put("state", "not_registered").toString()
    }
  }

  /** The UnifiedPush endpoint URL the distributor gave us (stored by
   *  PushServiceImpl.onNewEndpoint), or "" when not registered yet. JS sends
   *  it to the paired host (notify_register) so the PC pushes there. */
  @JavascriptInterface
  fun pushEndpoint(): String = prefs().getString("push_endpoint_url", null) ?: ""

  /** Stores the per-pairing key (base64) in private SharedPreferences -
   *  never the WebView's own storage - so background delivery can decrypt
   *  without a page loaded. Pass an empty string to clear it. */
  @JavascriptInterface
  fun setKey(keyB64: String) {
    prefs().edit().putString(PREF_KEY, keyB64).apply()
  }

  /** One queued action tapped from a notification while the app was
   *  closed (ask_user answer / permission allow-deny), or null. The JS
   *  side calls this on startup/resume, opens the right chat, and sends
   *  answer once the bridge (re)connects. Consumes the value (one-shot). */
  @JavascriptInterface
  fun takePendingAction(): String? {
    val stored = prefs().getString(PREF_PENDING_ACTION, null) ?: return null
    prefs().edit().remove(PREF_PENDING_ACTION).apply()
    return stored
  }

  private fun emit(js: String) {
    view.post { view.evaluateJavascript(js, null) }
  }
}
