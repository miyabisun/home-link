plugins { id("com.android.application") }

// The app's version is home-link's own, read from Cargo.toml so a release cannot leave the two apart.
val homeLinkVersion = Regex("""(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"""")
    .find(rootDir.resolve("../Cargo.toml").readText()) ?: error("no version in Cargo.toml")

android {
    namespace = "dev.miyabisun.homelink"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.miyabisun.homelink"
        minSdk = 36
        targetSdk = 37
        val (major, minor, patch) = homeLinkVersion.destructured
        // 0.1.48's code was 41; this grows with every version from there.
        versionCode = major.toInt() * 1_000_000 + minor.toInt() * 1_000 + patch.toInt()
        versionName = "$major.$minor.$patch"
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
