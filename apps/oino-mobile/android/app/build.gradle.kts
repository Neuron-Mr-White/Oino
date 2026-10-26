// Oino Mobile app module: NativeActivity host for liboino_mobile.so.
plugins {
    id("com.android.application")
}

android {
    namespace = "dev.oino.mobile"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.oino.mobile"
        minSdk = 26 // Vulkan 1.0 is mandatory from API 26.
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0-alpha.0"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
        // NativeActivity loads lib<nativeLibraryName>.so (the `oino_mobile` cdylib).
        manifestPlaceholders["nativeLibraryName"] = "oino_mobile"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Debug-signed so `assembleRelease` installs locally; release signing is
            // configured by the release workflow (integration).
            signingConfig = signingConfigs.getByName("debug")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            isDebuggable = true
            isJniDebuggable = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.directories.add("src/main/jniLibs")
        }
    }

    packaging {
        jniLibs {
            // cargo already strips release builds; keep symbols for debug backtraces.
            keepDebugSymbols += listOf("*/arm64-v8a/liboino_mobile.so")
            // NativeActivity dlopens the library from the APK's lib dir.
            useLegacyPackaging = true
        }
    }

    lint {
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    // NotificationCompat (GpuiNotifications, GpuiMediaSession).
    implementation("androidx.core:core:1.12.0")
    // SplashScreen compat (GpuiActivity holds the splash until native init).
    implementation("androidx.core:core-splashscreen:1.0.1")
    // MediaButtonReceiver (GpuiMediaSession, released by GpuiActivity).
    implementation("androidx.media:media:1.7.1")
}
