package dev.unipi.app

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.Settings
import android.text.TextUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.app.NotificationCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import kotlin.math.hypot

/**
 * UNI-200: the Unicrab "over other apps" (opt-in, Settings → Unicrab).
 *
 * A foreground service (an ongoing low-importance notification says it's on)
 * that hosts a small TYPE_APPLICATION_OVERLAY window: the crab
 * (UnicrabView, the same rig as the app) + a reply bubble that fades in and
 * out beside it. Drag to move, it snaps to the nearer side (remembered);
 * tap the crab → UniPi, tap a bubble → that chat (unipi://chat?host=&pid=).
 *
 * Fed two ways:
 * - while the app runs, the page pushes its mascot state + bubbles through
 *   window.UniPiOverlay.update (OverlayBridge → [feed]);
 * - while the app is in the background, the service polls the user's own
 *   ntfy topic itself (the same `GET /<topic>/json?poll=1` + `unipi1:`
 *   decrypt as lib/notifyPoll.ts, key from NotificationBridge's prefs),
 *   so the summarized reply pi sent shows up here too.
 * The overlay hides itself while UniPi is in front (the in-app floating
 * crab is there instead).
 */
class MascotOverlayService : Service() {
  companion object {
    const val CHANNEL_ID = "unipi-mascot"
    const val NOTIFICATION_ID = 7201
    const val PREFS = "unipi_mascot"
    const val ACTION_STOP = "dev.unipi.app.MASCOT_STOP"
    const val POLL_MS = 30_000L
    const val BUBBLE_MS = 6_000L

    @Volatile var instance: MascotOverlayService? = null
      private set

    fun canDraw(context: Context): Boolean = Build.VERSION.SDK_INT < Build.VERSION_CODES.M || Settings.canDrawOverlays(context)

    fun start(context: Context) {
      if (!canDraw(context)) return
      val intent = Intent(context, MascotOverlayService::class.java)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(intent) else context.startService(intent)
    }

    fun stop(context: Context) {
      context.stopService(Intent(context, MascotOverlayService::class.java))
    }

    /** From the page (OverlayBridge.update): {state, count, bubble:{id,text,link}|null}. */
    fun feed(json: String) {
      val service = instance ?: return
      service.main.post { service.applyFeed(JSONObject(json), fromApp = true) }
    }

    /** Which overlay state a decrypted push maps to (lib/needsYou.ts groupFor). */
    fun stateForKind(kind: String?): String = when (kind) {
      "agent_end", "agent_settled", "workflow_end" -> "done"
      null, "", "other" -> "idle"
      else -> "needs-you"
    }
  }

  private val main = Handler(Looper.getMainLooper())
  private var root: FrameLayout? = null
  private var crab: UnicrabView? = null
  private var badge: TextView? = null
  private var bubble: TextView? = null
  private var bubbleLink: String? = null
  private var params: WindowManager.LayoutParams? = null
  private var appInFront = false
  private var lastBubbleId: String? = null
  private var pollThread: Thread? = null
  @Volatile private var running = false

  private val lifecycle = object : DefaultLifecycleObserver {
    override fun onStart(owner: LifecycleOwner) { appInFront = true; root?.visibility = View.GONE }
    override fun onStop(owner: LifecycleOwner) { appInFront = false; root?.visibility = View.VISIBLE }
  }

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onCreate() {
    super.onCreate()
    instance = this
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean("on", false).apply()
      stopSelf()
      return START_NOT_STICKY
    }
    startInForeground()
    if (!canDraw(this)) {
      stopSelf()
      return START_NOT_STICKY
    }
    if (root == null) showOverlay()
    if (!running) startPolling()
    return START_STICKY
  }

  override fun onDestroy() {
    running = false
    pollThread?.interrupt()
    main.removeCallbacksAndMessages(null)
    main.post { ProcessLifecycleOwner.get().lifecycle.removeObserver(lifecycle) }
    root?.let { runCatching { (getSystemService(WINDOW_SERVICE) as WindowManager).removeView(it) } }
    root = null
    instance = null
    super.onDestroy()
  }

  private fun startInForeground() {
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && manager.getNotificationChannel(CHANNEL_ID) == null) {
      manager.createNotificationChannel(NotificationChannel(CHANNEL_ID, "UniPi - Unicrab over other apps", NotificationManager.IMPORTANCE_MIN).apply {
        description = "Shown while the Unicrab floats over other apps."
        setShowBadge(false)
      })
    }
    val open = PendingIntent.getActivity(this, 0, launchIntent(null), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    val stop = PendingIntent.getService(this, 1, Intent(this, MascotOverlayService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    val notification: Notification = NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.ic_dialog_info)
      .setContentTitle("Unicrab is floating")
      .setContentText("Shows your agents' replies over other apps")
      .setOngoing(true)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .setContentIntent(open)
      .addAction(0, "Turn off", stop)
      .build()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
  }

  private fun dp(v: Float): Int = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

  private fun launchIntent(link: String?): Intent {
    val intent = if (link != null) Intent(Intent.ACTION_VIEW, Uri.parse(link)).setPackage(packageName)
    else packageManager.getLaunchIntentForPackage(packageName) ?: Intent(this, MainActivity::class.java)
    return intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
  }

  private fun open(link: String?) {
    hideBubble()
    crab?.state = "idle"
    setCount(0)
    startActivity(launchIntent(link))
  }

  @SuppressLint("ClickableViewAccessibility")
  private fun showOverlay() {
    val wm = getSystemService(WINDOW_SERVICE) as WindowManager
    val size = dp(56f)
    val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
    val metrics = resources.displayMetrics
    val right = prefs.getString("side", "right") == "right"
    val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE
    val lp = WindowManager.LayoutParams(
      WindowManager.LayoutParams.WRAP_CONTENT,
      WindowManager.LayoutParams.WRAP_CONTENT,
      type,
      WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
      PixelFormat.TRANSLUCENT,
    ).apply {
      gravity = Gravity.TOP or Gravity.START
      x = if (right) metrics.widthPixels - size - dp(6f) else dp(6f)
      y = (prefs.getFloat("y", 0.3f) * (metrics.heightPixels - size)).toInt()
    }

    val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
    val bubbleView = TextView(this).apply {
      setTextColor(Color.parseColor("#F5EFE6"))
      textSize = 14f
      maxLines = 3
      ellipsize = TextUtils.TruncateAt.END
      maxWidth = dp(240f)
      setPadding(dp(12f), dp(8f), dp(12f), dp(8f))
      background = GradientDrawable().apply { setColor(Color.parseColor("#1C1917")); cornerRadius = dp(16f).toFloat(); setStroke(dp(1f), Color.parseColor("#F07818")) }
      alpha = 0f
      visibility = View.GONE
      setOnClickListener { open(bubbleLink) }
    }
    val button = FrameLayout(this).apply {
      background = GradientDrawable().apply { setColor(Color.argb(184, 20, 18, 16)); cornerRadius = dp(18f).toFloat() }
      layoutParams = LinearLayout.LayoutParams(size, size)
    }
    val crabView = UnicrabView(this)
    button.addView(crabView, FrameLayout.LayoutParams(dp(46f), dp(46f), Gravity.CENTER))
    val badgeView = TextView(this).apply {
      setTextColor(Color.parseColor("#181818"))
      textSize = 10f
      gravity = Gravity.CENTER
      minWidth = dp(16f)
      setPadding(dp(3f), 0, dp(3f), 0)
      background = GradientDrawable().apply { setColor(Color.parseColor("#F07818")); cornerRadius = dp(8f).toFloat() }
      visibility = View.GONE
    }
    button.addView(badgeView, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, dp(16f), Gravity.TOP or Gravity.END))

    fun layoutFor(rightSide: Boolean) {
      row.removeAllViews()
      val gap = LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { if (rightSide) rightMargin = dp(8f) else leftMargin = dp(8f) }
      if (rightSide) { row.addView(bubbleView, gap); row.addView(button) } else { row.addView(button); row.addView(bubbleView, gap) }
    }
    layoutFor(right)

    // Drag (snap to the nearer side, remember), tap → app, semi-transparent when idle.
    val slop = ViewConfiguration.get(this).scaledTouchSlop
    var downX = 0f; var downY = 0f; var startX = 0; var startY = 0; var dragging = false
    button.setOnTouchListener { _, e ->
      when (e.actionMasked) {
        MotionEvent.ACTION_DOWN -> { downX = e.rawX; downY = e.rawY; startX = lp.x; startY = lp.y; dragging = false; row.alpha = 1f; true }
        MotionEvent.ACTION_MOVE -> {
          if (!dragging && hypot(e.rawX - downX, e.rawY - downY) > slop) dragging = true
          if (dragging) { lp.x = startX + (e.rawX - downX).toInt(); lp.y = startY + (e.rawY - downY).toInt(); runCatching { wm.updateViewLayout(row.parent as View, lp) } }
          true
        }
        MotionEvent.ACTION_UP -> {
          if (dragging) {
            val w = resources.displayMetrics.widthPixels
            val h = resources.displayMetrics.heightPixels
            val toRight = lp.x + size / 2 > w / 2
            layoutFor(toRight)
            lp.x = if (toRight) w - (row.width.takeIf { bubbleView.visibility == View.VISIBLE } ?: size) - dp(6f) else dp(6f)
            lp.y = lp.y.coerceIn(0, h - size)
            runCatching { wm.updateViewLayout(row.parent as View, lp) }
            prefs.edit().putString("side", if (toRight) "right" else "left").putFloat("y", lp.y.toFloat() / (h - size).coerceAtLeast(1)).apply()
          } else open(null)
          scheduleIdleFade()
          true
        }
        MotionEvent.ACTION_CANCEL -> { scheduleIdleFade(); true }
        else -> false
      }
    }

    val container = FrameLayout(this).apply { addView(row) }
    wm.addView(container, lp)
    root = container
    crab = crabView
    badge = badgeView
    bubble = bubbleView
    params = lp
    crabView.state = prefs.getString("state", "idle") ?: "idle"
    setCount(prefs.getInt("count", 0))
    ProcessLifecycleOwner.get().lifecycle.addObserver(lifecycle)
    scheduleIdleFade()
  }

  private val fade = Runnable { if (bubble?.visibility != View.VISIBLE && crab?.state != "needs-you") root?.animate()?.alpha(0.5f)?.setDuration(400)?.start() }
  private fun scheduleIdleFade() {
    root?.alpha = 1f
    main.removeCallbacks(fade)
    main.postDelayed(fade, 3_000)
  }

  private fun setCount(n: Int) {
    getSharedPreferences(PREFS, MODE_PRIVATE).edit().putInt("count", n).apply()
    badge?.apply {
      text = if (n > 9) "9+" else n.toString()
      visibility = if (n > 0) View.VISIBLE else View.GONE
    }
  }

  private val hideBubbleRunnable = Runnable { hideBubble() }
  private fun hideBubble() {
    val b = bubble ?: return
    b.animate().alpha(0f).setDuration(350).withEndAction { b.visibility = View.GONE }.start()
  }

  private fun showBubble(id: String, text: String, link: String?) {
    if (id == lastBubbleId) return
    lastBubbleId = id
    val b = bubble ?: return
    bubbleLink = link
    b.text = text
    b.visibility = View.VISIBLE
    b.alpha = 0f
    b.translationY = dp(4f).toFloat()
    b.animate().alpha(1f).translationY(0f).setDuration(350).start()
    scheduleIdleFade()
    // A right-side crab grows leftwards: keep its right edge on the screen edge.
    params?.let { lp ->
      val right = getSharedPreferences(PREFS, MODE_PRIVATE).getString("side", "right") == "right"
      if (right) b.post {
        val wm = getSystemService(WINDOW_SERVICE) as WindowManager
        lp.x = resources.displayMetrics.widthPixels - (root?.width ?: 0) - dp(6f)
        runCatching { wm.updateViewLayout(root, lp) }
      }
    }
    main.removeCallbacks(hideBubbleRunnable)
    main.postDelayed(hideBubbleRunnable, BUBBLE_MS)
  }

  private fun applyFeed(feed: JSONObject, fromApp: Boolean) {
    val state = feed.optString("state", "idle")
    crab?.state = state
    getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString("state", state).apply()
    if (feed.has("count")) setCount(feed.optInt("count"))
    val b = feed.optJSONObject("bubble")
    if (b != null && (!fromApp || !appInFront)) showBubble(b.optString("id"), b.optString("text"), b.optString("link").ifEmpty { null })
  }

  // ── Background feed: the user's ntfy topic (lib/notifyPoll.ts, natively) ──

  private fun startPolling() {
    running = true
    pollThread = Thread {
      while (running) {
        try {
          if (!appInFront) pollOnce()
          Thread.sleep(POLL_MS)
        } catch (e: InterruptedException) {
          return@Thread
        } catch (e: Exception) {
          // network / parse error: try again next round
          runCatching { Thread.sleep(POLL_MS) }
        }
      }
    }.apply { isDaemon = true; name = "unipi-mascot-poll"; start() }
  }

  private fun pollOnce() {
    val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
    val server = prefs.getString("server", "")?.trimEnd('/') ?: ""
    val topic = prefs.getString("topic", "") ?: ""
    if (server.isEmpty() || topic.isEmpty()) return
    val key = getSharedPreferences(NotificationBridge.PREFS_NAME, MODE_PRIVATE).getString(NotificationBridge.PREF_KEY, null) ?: return
    val since = prefs.getString("since", null) ?: "120s"
    val conn = URL("$server/${Uri.encode(topic)}/json?poll=1&since=${Uri.encode(since)}").openConnection() as HttpURLConnection
    conn.connectTimeout = 10_000
    conn.readTimeout = 15_000
    prefs.getString("token", "")?.takeIf { it.isNotEmpty() }?.let { conn.setRequestProperty("Authorization", "Bearer $it") }
    val body = try {
      if (conn.responseCode !in 200..299) return
      conn.inputStream.bufferedReader().use { it.readText() }
    } finally {
      conn.disconnect()
    }
    var lastId: String? = null
    var newest: JSONObject? = null
    var added = 0
    for (line in body.lineSequence()) {
      val entry = runCatching { JSONObject(line.trim()) }.getOrNull() ?: continue
      if (entry.optString("event") != "message") continue
      lastId = entry.optString("id").ifEmpty { lastId }
      val message = entry.optString("message")
      if (!NotifyCrypto.isEncryptedEnvelope(message)) continue
      val payload = runCatching { NotifyCrypto.decrypt(key, message) }.getOrNull() ?: continue
      val link = buildString {
        append("unipi://chat?")
        payload.host?.let { append("host=").append(Uri.encode(it)) }
        payload.pid?.let { append("&pid=").append(it) }
      }
      val text = if (payload.body.isBlank() || payload.body == "Tap to open in UniPi") payload.title else "${payload.title}: ${payload.body}"
      newest = JSONObject().put("state", stateForKind(payload.kind)).put("bubble", JSONObject().put("id", entry.optString("id")).put("text", text.take(140)).put("link", link))
      added += 1
    }
    lastId?.let { prefs.edit().putString("since", it).apply() }
    val feed = newest ?: return
    val count = prefs.getInt("count", 0) + added
    feed.put("count", count)
    main.post { applyFeed(feed, fromApp = false) }
  }
}

/**
 * window.UniPiOverlay — the page's handle on the overlay (lib/mascotFeed.ts
 * OverlayBridge): permission check + the system "Display over other apps"
 * page, start/stop, the live feed, and the ntfy settings the service polls.
 */
class OverlayBridge(private val activity: android.app.Activity) {
  @android.webkit.JavascriptInterface
  fun canDraw(): Boolean = MascotOverlayService.canDraw(activity)

  @android.webkit.JavascriptInterface
  fun requestPermission() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return
    activity.runOnUiThread {
      activity.startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${activity.packageName}")))
    }
  }

  @android.webkit.JavascriptInterface
  fun start() {
    activity.getSharedPreferences(MascotOverlayService.PREFS, android.content.Context.MODE_PRIVATE).edit().putBoolean("on", true).apply()
    activity.runOnUiThread { MascotOverlayService.start(activity) }
  }

  @android.webkit.JavascriptInterface
  fun stop() {
    activity.getSharedPreferences(MascotOverlayService.PREFS, android.content.Context.MODE_PRIVATE).edit().putBoolean("on", false).apply()
    activity.runOnUiThread { MascotOverlayService.stop(activity) }
  }

  @android.webkit.JavascriptInterface
  fun running(): Boolean = MascotOverlayService.instance != null

  @android.webkit.JavascriptInterface
  fun update(json: String) {
    runCatching { MascotOverlayService.feed(json) }
  }

  /** {serverUrl, topic, token} or {} (notifications off): what the service polls in the background. */
  @android.webkit.JavascriptInterface
  fun configure(json: String) {
    val cfg = runCatching { JSONObject(json) }.getOrNull() ?: return
    activity.getSharedPreferences(MascotOverlayService.PREFS, android.content.Context.MODE_PRIVATE).edit()
      .putString("server", cfg.optString("serverUrl"))
      .putString("topic", cfg.optString("topic"))
      .putString("token", cfg.optString("token"))
      .apply()
  }
}
