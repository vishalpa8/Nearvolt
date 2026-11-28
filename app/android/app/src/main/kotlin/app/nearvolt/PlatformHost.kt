package app.nearvolt

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.Uri
import android.net.wifi.ScanResult
import android.net.wifi.WifiManager
import android.nfc.NfcAdapter
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.Parcelable
import android.os.PowerManager
import android.os.SystemClock
import android.provider.Settings
import android.widget.Toast
import com.google.android.gms.common.moduleinstall.ModuleInstall
import com.google.android.gms.common.moduleinstall.ModuleInstallRequest
import com.google.mlkit.common.MlKitException
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withContext
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import java.io.File
import java.util.Locale

/** Implements the Pigeon interface by delegating to one small class per radio. */
class PlatformHost(private val activity: MainActivity, private val events: NearvoltEvents) : NearvoltPlatform {
    private val context: Context = activity.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Radios call back on their own threads; Flutter must be called on the main one. */
    private val emit: (PlatformEvent) -> Unit = { event -> scope.launch { runCatching { events.onEvent(event) } } }

    private val wifiDirect = WifiDirect(context)
    private val nfc = Nfc(activity, emit)
    private val usb = Usb(context, emit)
    private val updater = Updater(activity, emit)

    init {
        if (nearbyGranted) scope.launch { wifiDirect.removeLeftover() }
    }

    // Created on first use: if Google's scanner ever fails, only Scan code is affected.
    private val modules by lazy { ModuleInstall.getClient(activity) }
    private val scanner by lazy {
        GmsBarcodeScanning.getClient(
            activity,
            GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build(),
        )
    }

    private val prefs = context.getSharedPreferences("nearvolt", Context.MODE_PRIVATE)
    private val wifi = context.getSystemService(WifiManager::class.java)
    private val location = context.getSystemService(LocationManager::class.java)

    private val name: String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: friendlyModel()
    private val kind: String =
        if (context.resources.configuration.smallestScreenWidthDp >= 600) "tablet" else "phone"

    /** A phone that was never named is I2212 or SM-S938B: with its maker it means something to the other person. */
    private fun friendlyModel(): String {
        val maker = Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
        return if (Build.MODEL.startsWith(maker, ignoreCase = true)) Build.MODEL else "$maker ${Build.MODEL}"
    }

    override fun deviceInfo() = DeviceInfo(
        name = name,
        kind = kind,
        hasNfc = NfcAdapter.getDefaultAdapter(context) != null,
        dataDir = context.filesDir.absolutePath,
        downloadDir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Nearvolt").absolutePath,
        journalDir = File(context.filesDir, "journals").absolutePath,
        versionName = context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "",
    )

    override fun log(tag: String, message: String) = AppLog.write(tag, message)

    override suspend fun linkReport(): String = listOfNotNull(
        routerReport(),
        if (nearbyGranted) runCatching { wifiDirect.report() }.getOrNull() else null,
        phoneReport(),
    ).joinToString(" · ")

    /** The phone's own Wi-Fi link to a router: what limits "same Wi-Fi" transfers. */
    @Suppress("DEPRECATION") // The current WifiInfo otherwise needs a network callback registered in advance.
    private fun routerReport(): String {
        if (!wifi.isWifiEnabled) return "Wi-Fi off"
        val info = wifi.connectionInfo
        if (info == null || info.frequency <= 0) return "Wi-Fi on, no network"
        val standard = when (info.wifiStandard) {
            ScanResult.WIFI_STANDARD_11N -> "Wi-Fi 4"
            ScanResult.WIFI_STANDARD_11AC -> "Wi-Fi 5"
            ScanResult.WIFI_STANDARD_11AX -> "Wi-Fi 6"
            ScanResult.WIFI_STANDARD_11BE -> "Wi-Fi 7"
            else -> "Wi-Fi"
        }
        return "network: $standard, ${WifiDirect.band(info.frequency)} (${info.frequency} MHz), " +
            "signal ${info.rssi} dBm, link ${info.txLinkSpeedMbps} Mbps out / ${info.rxLinkSpeedMbps} Mbps in"
    }

    /** What makes a phone slow itself down: battery saver and heat. */
    private fun phoneReport(): String {
        val power = context.getSystemService(PowerManager::class.java)
        val battery = context.getSystemService(BatteryManager::class.java)
        val level = battery?.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
        val heat = when (power?.currentThermalStatus) {
            PowerManager.THERMAL_STATUS_NONE -> "cool"
            PowerManager.THERMAL_STATUS_LIGHT -> "warm"
            PowerManager.THERMAL_STATUS_MODERATE -> "hot, may slow down"
            null -> "temperature unknown"
            else -> "very hot, slowing down"
        }
        return "battery $level%${if (battery?.isCharging == true) " charging" else ""}, " +
            "battery saver ${if (power?.isPowerSaveMode == true) "on" else "off"}, $heat"
    }

    override fun readiness(): Readiness {
        val missing = permissionsFor("nearby").filterNot(::granted)
        return Readiness(
            permission = missing.isEmpty(),
            permissionInSettings = missing.isNotEmpty() && prefs.getBoolean(ASKED, false) &&
                missing.none(activity::shouldShowRequestPermissionRationale),
            wifi = wifi.isWifiEnabled,
            location = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU || location.isLocationEnabled,
            nfc = NfcAdapter.getDefaultAdapter(context)?.isEnabled ?: true,
        )
    }

    override suspend fun fix(requirement: Requirement): Readiness {
        when (requirement) {
            Requirement.PERMISSIONS -> {
                val missing = (permissionsFor("nearby") + permissionsFor("notifications")).filterNot(::granted)
                if (missing.isNotEmpty()) {
                    prefs.edit().putBoolean(ASKED, true).apply()
                    activity.permissions.launch(missing.toTypedArray())
                }
            }
            Requirement.APP_SETTINGS -> activity.settings.launch(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
            )
            Requirement.WIFI -> {
                activity.settings.launch(Intent(Settings.Panel.ACTION_WIFI))
                settle { wifi.wifiState == WifiManager.WIFI_STATE_ENABLING }
            }
            Requirement.LOCATION -> activity.settings.launch(Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS))
            Requirement.NFC -> activity.settings.launch(Intent(Settings.ACTION_NFC_SETTINGS))
        }
        return readiness().also { AppLog.write("setup", "after $requirement: $it") }
    }

    /** A radio takes a moment to come up after the user turns it on. */
    private suspend fun settle(turningOn: () -> Boolean) {
        repeat(30) {
            if (!turningOn()) return
            delay(100)
        }
    }

    private fun granted(permission: String) = activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    /** Wi-Fi Direct stays off without it; the shared Wi-Fi network, QR codes and NFC still work, but slower. */
    private val nearbyGranted get() = permissionsFor("nearby").all(::granted)

    private fun permissionsFor(feature: String): List<String> = when (feature) {
        "nearby" -> when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU -> listOf(
                Manifest.permission.NEARBY_WIFI_DEVICES,
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

    // A pick replaced by a newer one just ends, as if the user had backed out of it.
    override suspend fun pickFiles(): PickResult? {
        val uris = try {
            activity.pickFiles.launch(arrayOf("*/*"))
        } catch (_: CancellationException) {
            return null
        }
        return if (uris.isEmpty()) null else withContext(Dispatchers.IO) { NativeBridge.fromDocuments(uris) }
    }

    override suspend fun pickFolder(): PickResult? {
        val tree = try {
            activity.pickFolder.launch(null)
        } catch (_: CancellationException) {
            return null
        } ?: return null
        return withContext(Dispatchers.IO) { NativeBridge.fromTree(tree) }
    }

    /**
     * Google's code scanner: no camera permission and nothing added to the app.
     * Play services downloads it the first time, so the scan waits for that.
     */
    override suspend fun scanQr(): String? = try {
        prepareScanner()
        scan(retry = true)
    } catch (_: CancellationException) {
        null
    } catch (e: Exception) {
        AppLog.write("scanner", "scan failed${(e as? MlKitException)?.let { " with code ${it.errorCode}" } ?: ""}")
        AppLog.error("scanner", e)
        throw FlutterError("scanner", "The code scanner didn't start. Try again, or tap the phones instead.")
    }

    /**
     * On some phones Google's scanner reports going back as an error instead of
     * a cancel, so an error once the camera has been showing counts as going
     * back. Only an error straight away means the scanner couldn't open; then
     * it's repaired and tried once more.
     */
    private suspend fun scan(retry: Boolean): String? {
        val started = SystemClock.elapsedRealtime()
        return try {
            scanUntilClosed()
        } catch (e: MlKitException) {
            val open = SystemClock.elapsedRealtime() - started
            if (open >= SCANNER_SHOWN_MS) {
                AppLog.write("scanner", "closed without a code after %.1f s (code ${e.errorCode})".format(Locale.US, open / 1000.0))
                return null
            }
            if (!retry) throw e
            AppLog.write("scanner", "couldn't open (code ${e.errorCode}); repairing and trying again")
            prepareScanner()
            delay(1_000)
            scan(retry = false)
        }
    }

    /**
     * Shows the scanner and waits for its answer. The scanner is its own screen: this app
     * pauses while it shows and resumes when it closes, and the answer follows within
     * moments. On some phones it closes without ever answering, which left the Send screen
     * stuck; so if no answer comes shortly after the app is back, that counts as going back.
     */
    private suspend fun scanUntilClosed(): String? = coroutineScope {
        val back = CompletableDeferred<Unit>()
        var paused = false
        var backAt = 0L
        val watcher = object : DefaultLifecycleObserver {
            override fun onPause(owner: LifecycleOwner) {
                paused = true
            }

            override fun onResume(owner: LifecycleOwner) {
                if (paused) {
                    backAt = SystemClock.elapsedRealtime()
                    back.complete(Unit)
                }
            }
        }
        activity.lifecycle.addObserver(watcher)
        try {
            val answer = async { scanner.startScan().await().rawValue }
            val silent = async {
                back.await()
                delay(SCANNER_ANSWER_GRACE_MS)
            }
            select<String?> {
                answer.onAwait {
                    silent.cancel()
                    // Measured, so the wait can be cut further once real scans show how late a code can be.
                    val late = if (backAt == 0L) "before the app was back" else "${SystemClock.elapsedRealtime() - backAt} ms after the app was back"
                    AppLog.write("scanner", "the scanner answered $late")
                    it
                }
                silent.onAwait {
                    AppLog.write("scanner", "the scanner closed without an answer; counting it as going back")
                    answer.cancel()
                    null
                }
            }
        } finally {
            activity.lifecycle.removeObserver(watcher)
        }
    }

    /**
     * Always asks Play services to install the scanner: when it's already there
     * this returns at once, and it fixes a scanner Play services wrongly reports
     * as ready.
     */
    private suspend fun prepareScanner() {
        val response = runCatching {
            modules.installModules(ModuleInstallRequest.newBuilder().addApi(scanner).build()).await()
        }.onFailure { AppLog.error("scanner", it) }.getOrNull() ?: return
        if (!response.areModulesAlreadyInstalled()) {
            Toast.makeText(context, "Getting the code scanner ready…", Toast.LENGTH_SHORT).show()
            waitForScanner()
        }
    }

    private suspend fun waitForScanner() {
        repeat(120) {
            if (modules.areModulesAvailable(scanner).await().areModulesAvailable()) return
            delay(500)
        }
        throw FlutterError("scanner", "The code scanner is still downloading. Try again in a moment.")
    }

    override suspend fun startReceiving(tapTicket: String, port: Long): ReceiveSetup {
        nfc.serve(tapTicket)
        val group = if (nearbyGranted) wifiDirect.createGroup() else null
        AppLog.write(
            "receive",
            "listening on port $port, Wi-Fi Direct group ${group?.let { "on ${it.frequency} MHz" } ?: "not created"}",
        )
        return ReceiveSetup(group)
    }

    override fun updateTickets(tapTicket: String, port: Long) = nfc.serve(tapTicket)

    override fun releaseSelection(token: Long) = NativeBridge.release(token)

    override fun stopAdvertising(senderAddress: String?) {
        nfc.stopServing()
        if (nearbyGranted) scope.launch { wifiDirect.removeGroupIfUnused(senderAddress) }
    }

    override fun stopReceiving() {
        nfc.stopServing()
        wifiDirect.removeGroup()
    }

    override fun startTapReading() = nfc.read()

    override fun stopTapReading() = nfc.stopReading()

    override fun watchUsb() = usb.watch()

    override suspend fun joinWifi(group: WifiGroup): String? {
        val started = SystemClock.elapsedRealtime()
        val address = if (nearbyGranted) wifiDirect.join(group) else null
        val seconds = "%.1f".format(Locale.US, (SystemClock.elapsedRealtime() - started) / 1000.0)
        AppLog.write(
            "send",
            if (address != null) {
                "joined the receiver's Wi-Fi Direct group in $seconds s, receiver at $address"
            } else {
                "couldn't join the receiver's Wi-Fi Direct group after $seconds s"
            },
        )
        return address
    }

    override fun leaveWifi() = wifiDirect.removeGroup()

    override fun startTransferService(title: String) = TransferService.start(context, title)

    override fun updateTransferService(text: String, percent: Long) = TransferService.update(context, text, percent.toInt())

    override fun stopTransferService() = TransferService.stop(context)

    override suspend fun checkForUpdate(): UpdateInfo? = updater.check()

    override suspend fun installUpdate(update: UpdateInfo) = updater.install(update)

    fun onResume() = nfc.onResume()

    /** Android opened the app for a USB cable. */
    fun onNewIntent(intent: Intent) = usb.handle(intent)

    fun close() {
        stopTapReading()
        stopReceiving()
        usb.close()
        scope.cancel()
    }

    private companion object {
        /** Set once the permission dialogs have been shown, to tell "never asked" from "refused for good". */
        const val ASKED = "asked_permissions"

        /** Longer than Google's scanner takes to open and fail; shorter than anyone takes to go back. */
        const val SCANNER_SHOWN_MS = 1_500L

        /** How long the scanner may take to answer once the app is back in front. */
        const val SCANNER_ANSWER_GRACE_MS = 300L
    }
}

/** A typed extra, through whichever API this Android version has. */
inline fun <reified T : Parcelable> Intent.parcelable(name: String): T? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        getParcelableExtra(name, T::class.java)
    } else {
        @Suppress("DEPRECATION")
        getParcelableExtra(name)
    }
