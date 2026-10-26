// Oino Mobile ↔ Rust (liboino_mobile.a) for the iOS host.
//
// `oino_ios_register_app` (oino-mobile) prepares ~/Documents/oino, the tokio runtime and the
// embedded oino-core host, and registers the GPUI app callback; the `gpui_ios_*` functions
// come from the vendored gpui-mobile platform (apps/vendor/gpui-mobile/src/ios/ffi.rs).
#ifndef OINO_IOS_H
#define OINO_IOS_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

void oino_ios_register_app(void);

void gpui_ios_run_demo(void);
void *gpui_ios_get_window(void);
bool gpui_ios_request_frame(void *window_ptr);
void gpui_ios_set_frame_waker(void *window_ptr, void (*waker)(void *context), void *context);
void gpui_ios_will_enter_foreground(void *app_ptr);
void gpui_ios_did_become_active(void *app_ptr);
void gpui_ios_will_resign_active(void *app_ptr);
void gpui_ios_did_enter_background(void *app_ptr);
void gpui_ios_will_terminate(void *app_ptr);
void gpui_ios_handle_open_url(void *url_ptr);

#ifdef __cplusplus
}
#endif

#endif // OINO_IOS_H
