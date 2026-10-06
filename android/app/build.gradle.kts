plugins { id("com.android.application") }

android {
    namespace = "dev.miyabisun.homelink"
    compileSdk = 37
    defaultConfig {
        applicationId = "dev.miyabisun.homelink"
        minSdk = 37
        targetSdk = 37
        versionCode = 8
        versionName = "0.1.8"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The tailnet URL of the home-link API. Cleartext is permitted only for
        // the host in res/xml/network_security_config.xml.
        val url = (findProperty("homeLinkUrl") as String?) ?: "http://homeserver:5011"
        buildConfigField("String", "HOME_LINK_URL", "\"$url\"")
        // matterjs-server's BLE proxy endpoint; cleartext is permitted for its host as above.
        val ble = (findProperty("bleProxyUrl") as String?) ?: "ws://192.168.1.100:5580/ble"
        buildConfigField("String", "BLE_PROXY_URL", "\"$ble\"")
    }
    buildFeatures { buildConfig = true }
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
