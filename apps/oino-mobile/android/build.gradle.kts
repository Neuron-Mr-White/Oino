// Root build for the Oino Mobile Android host. The Rust library is built outside Gradle
// (`apps/oino-mobile/scripts/build.sh android`), which places it in app/src/main/jniLibs.
buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
    }
}

tasks.register("clean", Delete::class) {
    delete(rootProject.layout.buildDirectory)
}
