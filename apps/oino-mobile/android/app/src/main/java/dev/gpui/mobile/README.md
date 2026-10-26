Java glue from longbridge/gpui-mobile `example/android` (`c24a130`, Apache-2.0 option),
trimmed to the packages Oino enables (see apps/vendor/gpui-mobile/VENDOR.md). Classes for
disabled packages (audio, calendar, camera, contacts, image picker, in-app review, local
auth, location, maps, microphone, video player) are not copied; `GpuiPlatformView` returns
no view for `video_player` / `camera_preview`. The package name stays `dev.gpui.mobile`
because the Rust side looks the classes up by name.
