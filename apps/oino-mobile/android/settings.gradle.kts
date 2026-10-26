// Oino Mobile Android host (gpui-app §4): one module packaging liboino_mobile.so
// (built by `../scripts/build.sh android` via cargo-ndk) behind GpuiActivity.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "OinoMobile"
include(":app")
