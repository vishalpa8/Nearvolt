package app.nearvolt

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.nfc.NfcAdapter
import android.os.Build
import android.os.Environment
import android.provider.Settings
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Implements the Pigeon interface by delegating to one small class per radio. */
class PlatformHost(private val activity: MainActivity, private val events: NearvoltEvents) : NearvoltPlatform {
    private val context: Context = activity.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Radios call back on their own threads; Flutter must be called on the main one. */
    private val emit: (PlatformEvent) -> Unit = { event -> scope.launch { runCatching { events.onEvent(event) } } }

    private val wifiDirect = WifiDirect(context)
    private val local = LocalDiscovery(context, emit)
    private val ble = Ble(context, emit)
    private val nfc = Nfc(activity, emit)
    private val updater = Updater(context, emit)

    private val name: String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL
    private val kind: String =
        if (context.resources.configuration.smallestScreenWidthDp >= 600) "tablet" else "phone"

    override fun deviceInfo() = DeviceInfo(
        name = name,
        kind = kind,
        hasNfc = NfcAdapter.getDefaultAdapter(context) != null,
        hasBluetooth = context.packageManager.hasSystemFeature(PackageManager.FEATURE_BLUETOOTH_LE),
        dataDir = context.filesDir.absolutePath,
        downloadDir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Nearvolt").absolutePath,
        journalDir = File(context.filesDir, "journals").absolutePath,
        versionName = context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "",
    )

    override suspend fun requestPermissions(features: List<String>): Boolean {
        val needed = features.flatMap(::permissionsFor).distinct()
            .filter { activity.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (needed.isEmpty()) return true
        return activity.permissions.launch(needed.toTypedArray()).values.all { it }
    }

    private fun permissionsFor(feature: String): List<String> = when (feature) {
        "nearby" -> when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU -> listOf(
                Manifest.permission.NEARBY_WIFI_DEVICES,
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_ADVERTISE,
                Manifest.permission.BLUETOOTH_CONNECT,
            )
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> listOf(
                Manifest.permission.ACCESS_FINE_LOCATION,
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_ADVERTISE,
                Manifest.permission.BLUETOOTH_CONNECT,
            )
            else -> listOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }
        "notifications" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            listOf(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            emptyList()
        }
        else -> emptyList()
    }

    override suspend fun pickFiles(): PickResult? {
        val uris = activity.pickFiles.launch(arrayOf("*/*"))
        return if (uris.isEmpty()) null else withContext(Dispatchers.IO) { NativeBridge.fromDocuments(uris) }
    }

    override suspend fun pickFolder(): PickResult? {
        val tree = activity.pickFolder.launch(null) ?: return null
        return withContext(Dispatchers.IO) { NativeBridge.fromTree(tree) }
    }

    /** Google's code scanner: no camera permission and nothing added to the app. */
    override suspend fun scanQr(): String? = suspendCancellableCoroutine { continuation ->
        val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
        GmsBarcodeScanning.getClient(activity, options).startScan()
            .addOnSuccessListener { continuation.resume(it.rawValue) }
            .addOnCanceledListener { continuation.resume(null) }
            .addOnFailureListener { continuation.resumeWithException(FlutterError("scanner", it.message)) }
    }

    override suspend fun startReceiving(privateTicket: String, publicTicket: String, port: Long): WifiGroup? {
        updateTickets(privateTicket, publicTicket, port)
        return wifiDirect.createGroup()
    }

    override fun updateTickets(privateTicket: String, publicTicket: String, port: Long) {
        nfc.serve(privateTicket)
        local.advertise(name, kind, publicTicket, port.toInt())
        ble.advertise(publicTicket)
    }

    override fun stopReceiving() {
        nfc.stopServing()
        local.stopAdvertising()
        ble.stopAdvertising()
        wifiDirect.removeGroup()
    }

    override fun startDiscovery() {
        local.browse()
        ble.scan()
        nfc.read()
    }

    override fun stopDiscovery() {
        local.stopBrowsing()
        ble.stopScan()
        nfc.stopReading()
    }

    override suspend fun readTicket(receiverId: String): String = ble.readTicket(receiverId)

    override suspend fun joinWifi(group: WifiGroup): Boolean = wifiDirect.join(group)

    override fun leaveWifi() = wifiDirect.removeGroup()

    override fun startTransferService(title: String) = TransferService.start(context, title)

    override fun updateTransferService(text: String, percent: Long) = TransferService.update(context, text, percent.toInt())

    override fun stopTransferService() = TransferService.stop(context)

    override suspend fun checkForUpdate(): UpdateInfo? = runCatching { updater.check() }.getOrNull()

    override suspend fun installUpdate(update: UpdateInfo) = updater.install(update)

    fun onResume() = nfc.onResume()

    fun close() {
        stopDiscovery()
        stopReceiving()
        scope.cancel()
    }
}
