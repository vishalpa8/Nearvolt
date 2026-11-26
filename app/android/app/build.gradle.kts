plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "app.nearvolt"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "app.nearvolt"
        // Android 11: Wi-Fi Direct joins without a pop-up (API 29) and the app can
        // write straight into Download/Nearvolt (API 30).
        minSdk = 30
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
}

// Builds the Rust engine (ffi crate) for every ABI into jniLibs before each build.
// NEARVOLT_ABIS narrows the list for faster local builds, e.g. "arm64-v8a".
val rustAbis = (System.getenv("NEARVOLT_ABIS") ?: "arm64-v8a,armeabi-v7a,x86_64").split(",")
val buildRust by tasks.registering(Exec::class) {
    description = "Builds the Nearvolt Rust engine for Android"
    workingDir = rootProject.projectDir.resolve("../..")
    val abis = rustAbis.flatMap { listOf("-t", it.trim()) }
    commandLine(
        listOf("cargo", "ndk", "--platform", "30", "-o", "app/android/app/src/main/jniLibs") +
            abis + listOf("build", "--release", "-p", "nearvolt-ffi"),
    )
}
tasks.named("preBuild") { dependsOn(buildRust) }
