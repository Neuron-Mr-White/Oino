# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# Our own WebView bridges (UniPiHaptics, UniPiUpdate): R8 must keep the
# @JavascriptInterface methods or release builds lose them.
-keepclassmembers class dev.unipi.app.** {
    @android.webkit.JavascriptInterface <methods>;
}
-keep class dev.unipi.app.UpdateBridge { *; }

# [phone-scan] QR scanning crashed release builds on the first camera frame
# (NPE in mlkit_vision_common zzkx.<init>: MlKitContext.get(SharedPrefManager)
# returned null). AGP 9 runs R8 in full mode, where `-keep class X` no longer
# keeps X's no-arg constructor. firebase-components only ships
# `-keep class * implements ComponentRegistrar`, so R8 removed the <init>() of
# ML Kit's CommonComponentRegistrar / VisionCommonRegistrar / BarcodeRegistrar
# (see build/outputs/mapping/*/usage.txt). ComponentDiscovery instantiates them
# by reflection from the manifest; without the constructor no ML Kit component
# was registered. Keep the constructors (and the registrars' getComponents).
-keep class * implements com.google.firebase.components.ComponentRegistrar {
    <init>();
    public java.util.List getComponents();
}
