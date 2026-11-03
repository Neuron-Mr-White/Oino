import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Release signing from the environment (scripts/release.sh sources it from
// the credentials folder; never committed). Without it, release builds are
// unsigned and debug builds use the debug key, as before.
val releaseKeystore: String? = System.getenv("UNIPI_ANDROID_KEYSTORE")
// QA/dev builds that install next to the released app: UNIPI_ANDROID_ID_SUFFIX=.dev
// gives package dev.unipi.oino.dev and the launcher name "Unipi" (own data, own signature).
val idSuffix: String? = System.getenv("UNIPI_ANDROID_ID_SUFFIX")?.takeIf { it.isNotBlank() }

android {
    compileSdk = 37
    signingConfigs {
        if (releaseKeystore != null) {
            create("release") {
                storeFile = file(releaseKeystore)
                storePassword = System.getenv("UNIPI_ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("UNIPI_ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("UNIPI_ANDROID_KEY_PASSWORD")
            }
        }
    }
    namespace = "dev.unipi.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        // Owner 2026-11-03: the installed id is dev.unipi.oino (matches desktop), so this app
        // installs next to the old UniPi (dev.unipi.app). The Kotlin namespace/package stays
        // dev.unipi.app (code only; Tauri's generated classes and JNI names live there).
        applicationId = "dev.unipi.oino"
        if (idSuffix != null) {
            applicationIdSuffix = idSuffix
            manifestPlaceholders["appLabel"] = "Unipi"
        } else {
            manifestPlaceholders["appLabel"] = "@string/app_name"
        }
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            if (releaseKeystore != null) signingConfig = signingConfigs.getByName("release")
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
    // UNI-200: the Unicrab rig (unicrab.sprite.json + unicrab.anim.json) — one copy, shared with the web renderer.
    sourceSets {
        getByName("main") {
            assets.srcDir("../../../../src/assets/unicrab")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    // UNI-161: UnifiedPush delivery (register with a distributor like the
    // ntfy app; receive our already AES-256-GCM-encrypted payload while
    // the app is closed). https://unifiedpush.org/developers/android/
    implementation("org.unifiedpush.android:connector:3.3.5")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
