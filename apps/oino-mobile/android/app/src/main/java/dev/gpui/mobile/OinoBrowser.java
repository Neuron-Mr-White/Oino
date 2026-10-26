package dev.gpui.mobile;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;

/**
 * Oino Mobile in-app browser (design.md A1.4): opens a URL as an Android Custom Tab.
 *
 * Uses the Custom Tabs intent protocol directly (the session extra marks the VIEW intent as a
 * custom tab for Chrome and other supporting browsers), so no androidx.browser dependency is
 * needed. Browsers without Custom Tabs open the URL normally.
 */
public final class OinoBrowser {
    private static final String EXTRA_SESSION = "android.support.customtabs.extra.SESSION";
    private static final String EXTRA_TOOLBAR_COLOR = "android.support.customtabs.extra.TOOLBAR_COLOR";
    private static final String EXTRA_COLOR_SCHEME = "androidx.browser.customtabs.extra.COLOR_SCHEME";
    private static final String EXTRA_TITLE_VISIBILITY = "android.support.customtabs.extra.TITLE_VISIBILITY";

    private OinoBrowser() {}

    /** Opens {@code url}; {@code dark} picks the toolbar colour. Returns false on failure. */
    public static boolean open(Activity activity, String url, boolean dark) {
        if (activity == null || url == null) return false;
        Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
        Bundle extras = new Bundle();
        extras.putBinder(EXTRA_SESSION, null);
        intent.putExtras(extras);
        intent.putExtra(EXTRA_TOOLBAR_COLOR, dark ? 0xFF0A0908 : 0xFFFBF8F4);
        intent.putExtra(EXTRA_COLOR_SCHEME, dark ? 2 : 1);
        intent.putExtra(EXTRA_TITLE_VISIBILITY, 1);
        try {
            activity.startActivity(intent);
            return true;
        } catch (ActivityNotFoundException e) {
            Log.w("OinoBrowser", "no browser for " + url);
            return false;
        }
    }
}
