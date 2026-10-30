package dev.unipi.app

import android.os.Bundle
import android.os.Build
import android.util.Log
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  private var page: WebView? = null
  private var insetsScript = ""
  // Keep the system splash (dark bg + Unicrab) up until the page has
  // parsed index.html, so it hands over straight to the in-app
  // animated splash instead of flashing an empty dark window.
  @Volatile private var pageReady = false
  private val launchedAt = android.os.SystemClock.uptimeMillis()
  // UNI-224: the app's own origin (http://tauri.localhost, or the dev server), seen once the splash is up.
  @Volatile private var appOrigin: String? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    val content = findViewById<android.view.View>(android.R.id.content)
    content.viewTreeObserver.addOnPreDrawListener(object : android.view.ViewTreeObserver.OnPreDrawListener {
      override fun onPreDraw(): Boolean {
        // Hard cap so a slow/failed page load never hangs on the splash.
        val ready = pageReady || android.os.SystemClock.uptimeMillis() - launchedAt > 4000
        if (ready) content.viewTreeObserver.removeOnPreDrawListener(this)
        return ready
      }
    })
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = false
      isAppearanceLightNavigationBars = false
    }
    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { _, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      val density = resources.displayMetrics.density
      insetsScript = """
        (() => {
          const root = document.documentElement;
          root.style.setProperty('--inset-top', '${bars.top / density}px');
          root.style.setProperty('--inset-bottom', '${bars.bottom / density}px');
          root.style.setProperty('--inset-left', '${bars.left / density}px');
          root.style.setProperty('--inset-right', '${bars.right / density}px');
          root.style.setProperty('--inset-ime', '${ime.bottom / density}px');
          // adjustResize: Android reports the keyboard here (the visual viewport no longer shrinks).
          window.unipiImeInset = ${ime.bottom / density};
          window.dispatchEvent(new Event('unipi-ime'));
          window.unipiDeviceModel = ${org.json.JSONObject.quote(Build.MODEL)};
        })();
      """.trimIndent()
      applyInsets()
      insets
    }
    ViewCompat.requestApplyInsets(window.decorView)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    page = webView
    // Dark page background before the first paint (no white flash).
    // The barcode scanner saves and restores this drawable around a
    // windowed scan; without one it would restore WHITE.
    webView.setBackgroundColor(android.graphics.Color.parseColor("#0A0908"))
    // Android draws its own fading scrollbar overlay on the WebView
    // whenever the native content height can exceed the view (even
    // when CSS keeps it non-scrollable); the terminal page never
    // needs page-level scrolling, so the overlay is just visual
    // noise (a persistent white line on the right edge). Disable it.
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false
    webView.overScrollMode = android.view.View.OVER_SCROLL_NEVER
    webView.scrollBarStyle = android.view.View.SCROLLBARS_INSIDE_OVERLAY
    webView.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> applyInsets() }
    // Crisp system haptics for the terminal toolbar (UNI-127): the
    // haptics plugin only offers a weak 50 ms vibration, and HyperOS
    // users expect the keyboard-style tick. Follows the system
    // touch-feedback setting; no VIBRATE permission needed.
    webView.addJavascriptInterface(HapticsBridge(webView), "UniPiHaptics")
    // In-app updates from unipi.nrn.one (UpdateBridge.kt).
    webView.addJavascriptInterface(UpdateBridge(this, webView), "UniPiUpdate")
    // UNI-161: local notifications + UnifiedPush registration (NotificationBridge.kt).
    webView.addJavascriptInterface(NotificationBridge(this, webView), "UniPiNotify")
    // UNI-169: download host files to the phone + open them in other apps (FilesBridge.kt).
    webView.addJavascriptInterface(FilesBridge(this), "UniPiFiles")
    // UNI-200: the Unicrab over other apps (MascotOverlayService.kt).
    webView.addJavascriptInterface(OverlayBridge(this), "UniPiOverlay")
    pollPageReady(webView, 0)
    webView.postDelayed({ measureInsets(); applyInsets() }, 1000)
    webView.postDelayed({ applyInsets() }, 2500)
  }

  override fun onResume() {
    super.onResume()
    addBackGuard()
    window.decorView.postDelayed({
      fun locate(view: android.view.View): WebView? {
        if (view is WebView) return view
        if (view is android.view.ViewGroup) {
          for (index in 0 until view.childCount) {
            val found = locate(view.getChildAt(index))
            if (found != null) return found
          }
        }
        return null
      }
      page = locate(window.decorView)
      ViewCompat.requestApplyInsets(window.decorView)
      applyInsets()
      page?.postDelayed({ measureInsets() }, 500)
    }, 1500)
  }

  private class HapticsBridge(private val view: WebView) {
    @android.webkit.JavascriptInterface
    fun tick(kind: String) {
      val constant = when (kind) {
        "tick" -> android.view.HapticFeedbackConstants.CLOCK_TICK
        else -> android.view.HapticFeedbackConstants.KEYBOARD_TAP
      }
      view.post { view.performHapticFeedback(constant) }
    }
  }

  private fun originOf(url: String?): String? {
    val uri = android.net.Uri.parse(url ?: return null)
    val scheme = uri.scheme ?: return null
    val host = uri.host ?: return null
    return if (uri.port > 0) "$scheme://$host:${uri.port}" else "$scheme://$host"
  }

  /**
   * UNI-224 safety net. If the WebView ever leaves the app (a previewed site
   * busting out of its iframe, a link that loaded over the app), our page and
   * its JS Back listener are gone, but Tauri's AppPlugin still forwards Back to
   * that dead listener, so Back did nothing and the app could not be left.
   * Re-added on every resume and once the page is up, so it is always the
   * newest callback and runs first (after the plugins'): off-origin it goes
   * back (or reloads the app); on the app it steps aside for AppPlugin.
   */
  private val backGuard = object : androidx.activity.OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val view = page
        val home = appOrigin
        val here = originOf(view?.url)
        if (view != null && home != null && here != null && here != home) {
          if (view.canGoBack()) view.goBack() else view.loadUrl("$home/")
          return
        }
        isEnabled = false
        onBackPressedDispatcher.onBackPressed()
        isEnabled = true
      }
  }

  private fun addBackGuard() {
    backGuard.remove()
    onBackPressedDispatcher.addCallback(this, backGuard)
  }

  /** Polls until the in-app splash element exists (index.html parsed). */
  private fun pollPageReady(webView: WebView, attempt: Int) {
    if (pageReady) return
    if (attempt > 80) { pageReady = true; return }
    webView.postDelayed({
      webView.evaluateJavascript("document.getElementById('splash') ? 1 : 0") { result ->
        if (result == "1") {
          appOrigin = originOf(webView.url)
          addBackGuard()
          // One more frame so the splash is actually painted.
          webView.postDelayed({ pageReady = true; window.decorView.invalidate() }, 80)
        } else {
          pollPageReady(webView, attempt + 1)
        }
      }
    }, 50)
  }

  private fun measureInsets() {
    page?.evaluateJavascript("""
      (() => {
        const probe = document.createElement('div');
        probe.style.cssText = 'padding-top:env(safe-area-inset-top);padding-bottom:env(safe-area-inset-bottom)';
        document.body.append(probe);
        const style = getComputedStyle(probe);
        const rootStyle = getComputedStyle(document.documentElement);
        const result = JSON.stringify({top:style.paddingTop,bottom:style.paddingBottom,nativeTop:rootStyle.getPropertyValue('--inset-top'),nativeBottom:rootStyle.getPropertyValue('--inset-bottom')});
        probe.remove(); return result;
      })()
    """.trimIndent()) { result -> Log.i("UniPiInsets", "WebView env insets: $result"); java.io.File(applicationInfo.dataDir, "insets-diagnostic.txt").writeText(result) }
  }

  private fun applyInsets() {
    if (insetsScript.isNotEmpty()) page?.evaluateJavascript(insetsScript, null)
  }
}
