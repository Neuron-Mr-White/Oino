//! [`MobilePlatform`] hooks backed by `gpui-mobile` and the host projects: soft keyboard,
//! safe areas, local notifications (`osNotify`, pending interactions), app badge, in-app
//! browser (Android Custom Tabs via `OinoBrowser.java`, iOS `SFSafariViewController` in
//! `main.m`), haptics, `oino://` deep links, paired hosts in `mobile.json` and system bar
//! colours. On desktop hosts (where the crate only type-checks) every hook is inert.

use std::path::PathBuf;
use std::sync::OnceLock;

use oino_ui::mobile::{MobilePlatform, RemoteHost};

/// The agent dir (`mobile.json` lives there), set once at boot.
static AGENT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Remembers the agent dir for the storage hooks.
pub(crate) fn set_agent_dir(dir: PathBuf) {
    let _ = AGENT_DIR.set(dir);
}

fn load_hosts() -> Vec<RemoteHost> {
    AGENT_DIR
        .get()
        .map(|d| crate::prefs::load_hosts(d))
        .unwrap_or_default()
}

fn save_hosts(hosts: &[RemoteHost]) {
    if let Some(d) = AGENT_DIR.get() {
        crate::prefs::save_hosts(d, hosts);
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
mod device {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use gpui_mobile::packages::{deeplink, notifications};
    use oino_ui::mobile::Insets;

    static LINKS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
    static LINKS_READY: AtomicBool = AtomicBool::new(false);
    static NOTIFY_READY: AtomicBool = AtomicBool::new(false);

    #[cfg(target_os = "android")]
    pub(super) fn safe_area() -> Insets {
        gpui_mobile::android::jni::platform()
            .and_then(|platform| platform.primary_window())
            .map(|window| {
                let i = window.safe_area_insets_logical();
                Insets {
                    top: i.top,
                    bottom: i.bottom,
                    left: i.left,
                    right: i.right,
                }
            })
            .unwrap_or_default()
    }

    #[cfg(target_os = "ios")]
    pub(super) fn safe_area() -> Insets {
        // (top, bottom, left, right) in points.
        let (top, bottom, left, right) = gpui_mobile::safe_area_insets();
        Insets {
            top,
            bottom,
            left,
            right,
        }
    }

    pub(super) fn notify(id: i32, title: &str, body: &str) {
        if !NOTIFY_READY.swap(true, Ordering::AcqRel)
            && let Err(error) = notifications::initialize()
        {
            log::warn!("notifications: {error}");
        }
        let n = notifications::Notification {
            id,
            title: title.to_owned(),
            body: body.to_owned(),
            channel: notifications::NotificationChannel {
                id: "oino".into(),
                name: "Oino".into(),
                description: "Sessions that need you and extension notifications".into(),
                importance: notifications::Importance::High,
            },
            payload: None,
        };
        if let Err(error) = notifications::show(&n) {
            log::warn!("notification: {error}");
        }
    }

    /// Deep links: the launch link once, then links delivered while running.
    pub(super) fn take_link() -> Option<String> {
        if !LINKS_READY.swap(true, Ordering::AcqRel) {
            deeplink::set_deep_link_handler(|url| {
                if let Ok(mut q) = LINKS.lock() {
                    q.push_back(url.to_owned());
                }
            });
            if let Ok(Some(url)) = deeplink::get_initial_link()
                && let Ok(mut q) = LINKS.lock()
            {
                q.push_back(url);
            }
        }
        LINKS.lock().ok().and_then(|mut q| q.pop_front())
    }

    pub(super) fn system_bars(dark: bool, color: u32) {
        gpui_mobile::set_system_chrome(&gpui_mobile::SystemChromeStyle {
            status_bar_color: Some(color),
            status_bar_style: if dark {
                gpui_mobile::StatusBarContentStyle::Light
            } else {
                gpui_mobile::StatusBarContentStyle::Dark
            },
            navigation_bar_color: Some(color),
        });
    }

    #[cfg(target_os = "android")]
    pub(super) fn open_browser(url: &str, dark: bool) -> bool {
        use gpui_mobile::android::jni as gj;
        use jni::objects::JValue;
        let result = gj::with_env(|env| {
            let activity = gj::activity(env)?;
            let cls = gj::find_app_class(env, "dev.gpui.mobile.OinoBrowser")?;
            let url = env.new_string(url).map_err(|e| e.to_string())?;
            let opened = env
                .call_static_method(
                    &cls,
                    jni::jni_str!("open"),
                    jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;Z)Z"),
                    &[
                        JValue::Object(&activity),
                        JValue::Object(&url),
                        JValue::Bool(dark),
                    ],
                )
                .and_then(jni::JValueOwned::z)
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
            Ok(opened)
        });
        result.unwrap_or_else(|error| {
            log::warn!("in-app browser: {error}");
            false
        })
    }

    #[cfg(target_os = "android")]
    pub(super) fn set_badge(_count: usize) {
        // Android draws the launcher dot from the posted notifications.
    }

    #[cfg(target_os = "android")]
    pub(super) fn haptic() {
        let _ = gpui_mobile::packages::vibration::haptic_feedback(
            gpui_mobile::packages::vibration::HapticFeedback::Light,
        );
    }

    #[cfg(target_os = "ios")]
    #[allow(
        unsafe_code,
        reason = "Objective-C entry points defined in ios/main.m (in-app browser, badge)"
    )]
    mod ffi {
        unsafe extern "C" {
            pub(super) fn oino_ios_open_browser(url: *const std::ffi::c_char, dark: bool) -> bool;
            pub(super) fn oino_ios_set_badge(count: std::ffi::c_long);
        }
    }

    #[cfg(target_os = "ios")]
    #[allow(
        unsafe_code,
        reason = "calls the main.m entry point with a valid C string"
    )]
    pub(super) fn open_browser(url: &str, dark: bool) -> bool {
        let Ok(c) = std::ffi::CString::new(url) else {
            return false;
        };
        // SAFETY: `c` is a valid NUL-terminated string for the duration of the call; the
        // Objective-C side copies it into an NSString before returning.
        unsafe { ffi::oino_ios_open_browser(c.as_ptr(), dark) }
    }

    #[cfg(target_os = "ios")]
    #[allow(unsafe_code, reason = "calls the main.m entry point")]
    pub(super) fn set_badge(count: usize) {
        let n = std::ffi::c_long::try_from(count).unwrap_or(std::ffi::c_long::MAX);
        // SAFETY: plain integer argument; main.m dispatches to the main queue.
        unsafe { ffi::oino_ios_set_badge(n) }
    }

    #[cfg(target_os = "ios")]
    pub(super) fn haptic() {
        let _ = gpui_mobile::packages::vibration::haptic_feedback(
            gpui_mobile::packages::vibration::HapticFeedback::Light,
        );
    }
}

/// The hooks for this platform (inert on desktop hosts, where the crate only type-checks).
#[must_use]
pub(crate) fn hooks() -> MobilePlatform {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        MobilePlatform {
            show_keyboard: gpui_mobile::show_keyboard,
            hide_keyboard: gpui_mobile::hide_keyboard,
            safe_area: device::safe_area,
            keyboard_height: gpui_mobile::keyboard_height,
            notify: device::notify,
            set_badge: device::set_badge,
            open_browser: device::open_browser,
            haptic: device::haptic,
            take_link: device::take_link,
            load_hosts,
            save_hosts,
            system_bars: device::system_bars,
        }
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        MobilePlatform {
            load_hosts,
            save_hosts,
            ..MobilePlatform::default()
        }
    }
}
