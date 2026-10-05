package dev.miyabisun.homelink

import android.app.Activity
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

sealed interface ScanResult {
    data class Read(val value: String) : ScanResult
    data object Cancelled : ScanResult
    data object Failed : ScanResult
}

fun interface QrScanner {
    fun scan(done: (ScanResult) -> Unit)
}

/** Google Code Scanner: Google Play services shows the camera UI, so no camera permission is needed. */
class GmsQrScanner(activity: Activity) : QrScanner {
    private val client = GmsBarcodeScanning.getClient(activity,
        GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())

    override fun scan(done: (ScanResult) -> Unit) {
        client.startScan()
            .addOnSuccessListener { done(it.rawValue?.let(ScanResult::Read) ?: ScanResult.Failed) }
            .addOnCanceledListener { done(ScanResult.Cancelled) }
            .addOnFailureListener { done(ScanResult.Failed) }
    }
}
