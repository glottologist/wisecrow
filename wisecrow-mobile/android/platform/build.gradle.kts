plugins {
    id("com.android.library") version "8.10.1"
    id("org.jetbrains.kotlin.android") version "2.0.20"
}

android {
    namespace = "org.wisecrow.mobile"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("com.squareup.okhttp3:okhttp:5.3.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("com.squareup.okhttp3:mockwebserver3:5.3.0")
    testImplementation("com.squareup.okhttp3:okhttp-tls:5.3.0")
    testImplementation("org.json:json:20260522")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
}
