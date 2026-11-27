package app.nearvolt

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.os.SystemClock
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import java.io.IOException
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * Finds receivers that aren't on the same network. A receiver advertises a
 * Nearvolt service and serves its public ticket as a readable characteristic;
 * the sender reads it only when the user taps that receiver.
 */
@SuppressLint("MissingPermission") // Callers request the nearby-devices permission first.
class Ble(private val context: Context, private val emit: (PlatformEvent) -> Unit) {
    private val manager = context.getSystemService(BluetoothManager::class.java)
    private val adapter get() = manager?.adapter?.takeIf { it.isEnabled }
    private var server: BluetoothGattServer? = null
    @Volatile private var ticket = ByteArray(0)
    private val found = mutableMapOf<String, BluetoothDevice>()

    /** When each receiver was last heard, so one that stops advertising leaves the list. */
    private val lastSeen = mutableMapOf<String, Long>()
    private val handler = Handler(Looper.getMainLooper())
    private val sweep = object : Runnable {
        override fun run() {
            expire(SystemClock.elapsedRealtime())
            handler.postDelayed(this, SWEEP_MS)
        }
    }

    /** Announces every receiver not heard for [GONE_AFTER_MS] as gone. */
    private fun expire(now: Long) {
        val gone = synchronized(found) {
            lastSeen.filterValues { now - it > GONE_AFTER_MS }.keys.toList().onEach {
                lastSeen.remove(it)
                found.remove(it)
            }
        }
        gone.forEach { emit(PlatformEvent(PlatformEventKind.RECEIVER_LOST, id = "$ID_PREFIX$it")) }
    }

    private val advertising = object : AdvertiseCallback() {
        override fun onStartFailure(errorCode: Int) {
            // A long device name can overflow the scan response; advertise without it.
            if (errorCode == ADVERTISE_FAILED_DATA_TOO_LARGE) startAdvertising(includeName = false)
        }
    }

    private val scanning = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) = try {
            found(result)
        } catch (e: SecurityException) {
            // The Bluetooth permission was taken away while scanning.
            AppLog.error("bluetooth", e)
        }

        private fun found(result: ScanResult) {
            val device = result.device
            val isNew = synchronized(found) {
                lastSeen[device.address] = SystemClock.elapsedRealtime()
                found.put(device.address, device) == null
            }
            if (isNew) {
                emit(
                    PlatformEvent(
                        PlatformEventKind.RECEIVER_FOUND,
                        id = "$ID_PREFIX${device.address}",
                        name = result.scanRecord?.deviceName ?: device.name ?: "Nearby device",
                        deviceKind = "phone",
                        link = "Bluetooth",
                    ),
                )
            }
        }
    }

    fun advertise(ticket: String) {
        this.ticket = ticket.toByteArray()
        if (server != null || adapter == null) return
        server = manager?.openGattServer(context, object : BluetoothGattServerCallback() {
            override fun onCharacteristicReadRequest(
                device: BluetoothDevice,
                requestId: Int,
                offset: Int,
                characteristic: BluetoothGattCharacteristic,
            ) {
                val value = this@Ble.ticket
                val slice = if (offset < value.size) value.copyOfRange(offset, value.size) else ByteArray(0)
                server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, slice)
            }
        })?.apply {
            val service = BluetoothGattService(SERVICE, BluetoothGattService.SERVICE_TYPE_PRIMARY)
            service.addCharacteristic(
                BluetoothGattCharacteristic(
                    TICKET,
                    BluetoothGattCharacteristic.PROPERTY_READ,
                    BluetoothGattCharacteristic.PERMISSION_READ,
                ),
            )
            addService(service)
        }
        startAdvertising(includeName = true)
    }

    private fun startAdvertising(includeName: Boolean) {
        val advertiser = adapter?.bluetoothLeAdvertiser ?: return
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setConnectable(true)
            .build()
        val data = AdvertiseData.Builder().addServiceUuid(ParcelUuid(SERVICE)).build()
        val response = AdvertiseData.Builder().setIncludeDeviceName(includeName).build()
        runCatching { advertiser.startAdvertising(settings, data, response, advertising) }
    }

    fun stopAdvertising() {
        runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(advertising) }
        server?.close()
        server = null
    }

    fun scan() {
        synchronized(found) {
            found.clear()
            lastSeen.clear()
        }
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE)).build()
        // Balanced: a receiver shows up within a couple of seconds, and the radio isn't
        // kept at full tilt for as long as the page is open.
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_BALANCED).build()
        runCatching { adapter?.bluetoothLeScanner?.startScan(listOf(filter), settings, scanning) }
        handler.removeCallbacks(sweep)
        handler.postDelayed(sweep, SWEEP_MS)
    }

    fun stopScan() {
        handler.removeCallbacks(sweep)
        runCatching { adapter?.bluetoothLeScanner?.stopScan(scanning) }
    }

    /** Connects to a receiver found by [scan] and reads its public ticket. */
    suspend fun readTicket(id: String): String = withTimeout(15_000) {
        val device = synchronized(found) { found[id.removePrefix(ID_PREFIX)] }
            ?: throw IOException("That device is no longer nearby")
        suspendCancellableCoroutine { continuation ->
            fun finish(gatt: BluetoothGatt, value: ByteArray?, status: Int) {
                gatt.close()
                if (!continuation.isActive) return
                if (status == BluetoothGatt.GATT_SUCCESS && value != null) {
                    continuation.resume(String(value))
                } else {
                    continuation.resumeWithException(IOException("Couldn't read the receiver's code over Bluetooth"))
                }
            }

            val gatt = device.connectGatt(context, false, object : BluetoothGattCallback() {
                override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                    when (newState) {
                        BluetoothProfile.STATE_CONNECTED -> gatt.requestMtu(517)
                        BluetoothProfile.STATE_DISCONNECTED -> finish(gatt, null, status)
                    }
                }

                override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
                    gatt.discoverServices()
                }

                override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                    val characteristic = gatt.getService(SERVICE)?.getCharacteristic(TICKET)
                    if (characteristic == null || !gatt.readCharacteristic(characteristic)) {
                        finish(gatt, null, BluetoothGatt.GATT_FAILURE)
                    }
                }

                override fun onCharacteristicRead(
                    gatt: BluetoothGatt,
                    characteristic: BluetoothGattCharacteristic,
                    value: ByteArray,
                    status: Int,
                ) = finish(gatt, value, status)

                @Deprecated("Called instead of the overload above before Android 13")
                @Suppress("DEPRECATION")
                override fun onCharacteristicRead(
                    gatt: BluetoothGatt,
                    characteristic: BluetoothGattCharacteristic,
                    status: Int,
                ) = finish(gatt, characteristic.value, status)
            }, BluetoothDevice.TRANSPORT_LE)
            continuation.invokeOnCancellation { gatt.close() }
        }
    }

    private companion object {
        val SERVICE: UUID = UUID.fromString("6e766f6c-7400-4b8e-9c4a-2f6a0e3b5a01")
        val TICKET: UUID = UUID.fromString("6e766f6c-7400-4b8e-9c4a-2f6a0e3b5a02")
        const val ID_PREFIX = "ble:"

        /** A receiver advertises several times a second; this long without a sign means it's gone. */
        const val GONE_AFTER_MS = 15_000L
        const val SWEEP_MS = 5_000L
    }
}
