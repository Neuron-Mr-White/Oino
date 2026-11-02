//! iOS only: stop WKWebView from insetting the page by the safe area itself.
//!
//! By default the webview's `UIScrollView` uses
//! `contentInsetAdjustmentBehavior = .automatic`, which shrinks the layout
//! viewport by the top + bottom safe areas (iPhone 11 Pro: 812 pt screen →
//! `innerHeight` 734 = 812 − 44 − 34) while `env(safe-area-inset-*)` still
//! reports 44/34. The CSS already pads with `env(safe-area-inset-*)`
//! (viewport-fit=cover), so the bottom nav ended 34 pt above the screen
//! edge *and* added its own 34 pt home-indicator pad: the "bottom padding
//! too big" on iPhone (measured in the simulator, 2026-10-11).
//! `.never` lets the page cover the whole screen; env() does the insets once.
//! WebKit only re-reads the obscured insets on a geometry change, so the
//! frame is nudged by 1 pt and back right after switching.
//!
//! Plain objc runtime calls (libobjc is already linked by wry), so no extra
//! crates and no change to the generated Xcode project.

use std::ffi::{c_char, c_void, CStr};

/// `UIScrollViewContentInsetAdjustmentNever`.
pub const CONTENT_INSET_ADJUSTMENT_NEVER: isize = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "objc")]
extern "C" {
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    // x86_64 (simulator) returns large structs through objc_msgSend_stret;
    // arm64 has no _stret variant (plain objc_msgSend uses x8).
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
}

unsafe fn sel(name: &CStr) -> *mut c_void {
    sel_registerName(name.as_ptr())
}

unsafe fn get_obj(obj: *mut c_void, name: &CStr) -> *mut c_void {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name))
}

unsafe fn get_isize(obj: *mut c_void, name: &CStr) -> isize {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name))
}

unsafe fn set_isize(obj: *mut c_void, name: &CStr, value: isize) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, isize) =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name), value)
}

unsafe fn set_bool(obj: *mut c_void, name: &CStr, value: bool) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, bool) =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name), value)
}

#[cfg(target_arch = "x86_64")]
unsafe fn get_rect(obj: *mut c_void, name: &CStr) -> CGRect {
    let f: unsafe extern "C" fn(*mut CGRect, *mut c_void, *mut c_void) =
        std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
    let mut rect = CGRect::default();
    f(&mut rect, obj, sel(name));
    rect
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn get_rect(obj: *mut c_void, name: &CStr) -> CGRect {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGRect =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name))
}

unsafe fn set_rect(obj: *mut c_void, name: &CStr, rect: CGRect) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, CGRect) =
        std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel(name), rect)
}

/// `wkwebview.scrollView.contentInsetAdjustmentBehavior = .never` (once;
/// later calls are no-ops), then a 1 pt frame nudge so WebKit re-lays the
/// page out at the full screen height. `wkwebview` is the `WKWebView*`
/// from `PlatformWebview::inner()`; null is ignored. Main thread only
/// (`Webview::with_webview` runs the closure there).
pub fn disable_safe_area_content_inset(wkwebview: *mut c_void) {
    if wkwebview.is_null() {
        return;
    }
    unsafe {
        let scroll_view = get_obj(wkwebview, c"scrollView");
        if scroll_view.is_null()
            || get_isize(scroll_view, c"contentInsetAdjustmentBehavior")
                == CONTENT_INSET_ADJUSTMENT_NEVER
        {
            return;
        }
        set_isize(
            scroll_view,
            c"setContentInsetAdjustmentBehavior:",
            CONTENT_INSET_ADJUSTMENT_NEVER,
        );
        set_bool(
            scroll_view,
            c"setAutomaticallyAdjustsScrollIndicatorInsets:",
            false,
        );
        let frame = get_rect(wkwebview, c"frame");
        set_rect(
            wkwebview,
            c"setFrame:",
            CGRect {
                h: frame.h - 1.0,
                ..frame
            },
        );
        set_rect(wkwebview, c"setFrame:", frame);
    }
}

/// UNI-263: open `url` in Safari (`[UIApplication.sharedApplication
/// openURL:options:completionHandler:]`). Main thread only. Used for the
/// "new version — open oino.dev/download" notice and other web links.
pub fn open_in_safari(url: &str) -> bool {
    let Ok(c_url) = std::ffi::CString::new(url) else {
        return false;
    };
    unsafe {
        let ns_string = objc_getClass(c"NSString".as_ptr());
        let ns_url = objc_getClass(c"NSURL".as_ptr());
        let ns_dict = objc_getClass(c"NSDictionary".as_ptr());
        let ui_app = objc_getClass(c"UIApplication".as_ptr());
        if ns_string.is_null() || ns_url.is_null() || ns_dict.is_null() || ui_app.is_null() {
            return false;
        }
        let with_utf8: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *const c_char,
        ) -> *mut c_void = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let string = with_utf8(ns_string, sel(c"stringWithUTF8String:"), c_url.as_ptr());
        let with_obj: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let nsurl = with_obj(ns_url, sel(c"URLWithString:"), string);
        if nsurl.is_null() {
            return false;
        }
        let shared = get_obj(ui_app, c"sharedApplication");
        let options = get_obj(ns_dict, c"dictionary");
        let open: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        open(
            shared,
            sel(c"openURL:options:completionHandler:"),
            nsurl,
            options,
            std::ptr::null_mut(),
        );
        true
    }
}
