//! [`MobilePlatform`] hooks backed by `gpui-mobile` (soft keyboard, safe areas).

#[cfg(any(target_os = "android", target_os = "ios"))]
use oino_ui::mobile::Insets;
use oino_ui::mobile::MobilePlatform;

#[cfg(target_os = "android")]
fn safe_area() -> Insets {
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
fn safe_area() -> Insets {
    // (top, bottom, left, right) in points.
    let (top, bottom, left, right) = gpui_mobile::safe_area_insets();
    Insets {
        top,
        bottom,
        left,
        right,
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
            safe_area,
            keyboard_height: gpui_mobile::keyboard_height,
        }
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        MobilePlatform::default()
    }
}
