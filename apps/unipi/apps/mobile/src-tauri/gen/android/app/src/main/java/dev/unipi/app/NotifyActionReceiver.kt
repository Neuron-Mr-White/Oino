package dev.unipi.app

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import org.json.JSONObject

/**
 * UNI-161 - a tap on a notification action button (an ask_user option, a
 * permission allow/deny, or the ongoing "Running" notification's Stop
 * button) while the app may not be running. Stashes the action for the JS
 * side to read via NotificationBridge.takePendingAction() on the next
 * startup/resume, then opens the app via the chat deep link so the user
 * sees the answer go through.
 */
class NotifyActionReceiver : BroadcastReceiver() {
  companion object {
    const val EXTRA_NOTIFICATION_ID = "notification_id"
    const val EXTRA_ACTION = "action"
  }

  override fun onReceive(context: Context, intent: Intent) {
    val notificationId = intent.getIntExtra(EXTRA_NOTIFICATION_ID, -1)
    val action = intent.getStringExtra(EXTRA_ACTION) ?: return

    val prefs = context.getSharedPreferences(NotificationBridge.PREFS_NAME, Context.MODE_PRIVATE)
    val pending = JSONObject().put("notificationId", notificationId).put("action", action)
    prefs.edit().putString(NotificationBridge.PREF_PENDING_ACTION, pending.toString()).apply()

    if (notificationId >= 0) {
      ContextCompat.getSystemService(context, NotificationManager::class.java)?.cancel(notificationId)
    }

    val openApp = Intent(Intent.ACTION_VIEW, android.net.Uri.parse("unipi://chat"))
      .setPackage(context.packageName)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    context.startActivity(openApp)
  }
}
