plugins { id("com.android.application") }

android {
    namespace = "dev.miyabisun.homelink"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.miyabisun.homelink"
        minSdk = 36
        targetSdk = 37
        versionCode = 30
        versionName = "0.1.35"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
dependencies {
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20260814")
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
