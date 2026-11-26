package app.nearvolt

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import androidx.core.content.ContextCompat
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

private typealias Transfer = (buffer: ByteArray, offset: Int, length: Int) -> Int

/**
 * Phone-to-phone USB through Android Open Accessory. The phone powering the
 * cable (the USB host) switches the other phone into accessory mode; then each
 * side pumps the cable through a socket pair whose other end the engine owns,
 * and the engine multiplexes its streams over it.
 *
 * Android asks once on each phone to open Nearvolt for the cable; the host also
 * asks for access to the other phone each time it is plugged in.
 */
class Usb(private val context: Context, private val emit: (PlatformEvent) -> Unit) {
    private val manager = context.getSystemService(UsbManager::class.java)
    private var watching = false

    @Volatile
    private var link: Pump? = null

    /** The other phone's name, learned before it reappears in accessory mode. */
    @Volatile
    private var peerName: String? = null

    private val events = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) = handle(intent)
    }

    /** Starts noticing cables, including one already plugged in. */
    fun watch() {
        if (watching) return
        watching = true
        val filter = IntentFilter().apply {
            addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
            addAction(UsbManager.ACTION_USB_ACCESSORY_DETACHED)
            addAction(PERMISSION)
        }
        ContextCompat.registerReceiver(context, events, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        guarded {
            manager.accessoryList?.forEach(::onAccessory)
            manager.deviceList.values.forEach { onDevice(it, ask = true) }
        }
    }

    fun close() {
        if (watching) context.unregisterReceiver(events)
        watching = false
        link?.close()
    }

    /** Attach, detach and permission broadcasts, and the activity opened for a cable. */
    fun handle(intent: Intent) = guarded {
        when (intent.action) {
            UsbManager.ACTION_USB_ACCESSORY_ATTACHED -> intent.parcelable<UsbAccessory>(UsbManager.EXTRA_ACCESSORY)?.let(::onAccessory)
            UsbManager.ACTION_USB_DEVICE_ATTACHED -> intent.parcelable<UsbDevice>(UsbManager.EXTRA_DEVICE)?.let { onDevice(it, ask = false) }
            UsbManager.ACTION_USB_ACCESSORY_DETACHED -> link?.takeIf { it.key == ACCESSORY }?.close()
            UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                val name = intent.parcelable<UsbDevice>(UsbManager.EXTRA_DEVICE)?.deviceName
                link?.takeIf { it.key == name }?.close()
            }
            PERMISSION -> if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) {
                intent.parcelable<UsbAccessory>(UsbManager.EXTRA_ACCESSORY)?.let(::onAccessory)
                intent.parcelable<UsbDevice>(UsbManager.EXTRA_DEVICE)?.let { onDevice(it, ask = false) }
            }
        }
    }

    /** This phone is the accessory: the other phone switched it over. */
    private fun onAccessory(accessory: UsbAccessory) {
        if (link != null || accessory.manufacturer != IDENTITY[0] || accessory.model != IDENTITY[1]) return
        if (!manager.hasPermission(accessory)) return manager.requestPermission(accessory, permissionIntent())
        val file = manager.openAccessory(accessory) ?: return
        val input = FileInputStream(file.fileDescriptor)
        val output = FileOutputStream(file.fileDescriptor)
        start(
            Pump(
                key = ACCESSORY,
                readBuffer = ACCESSORY_READ,
                read = input::read,
                write = { buffer, offset, length -> output.write(buffer, offset, length).let { length } },
                release = file::close,
            ),
            host = false,
            name = null,
        )
    }

    /**
     * This phone is the host. A phone in accessory mode gets access through the
     * manifest's device filter when Android opens the app for it, so only a
     * device found at startup asks here.
     */
    private fun onDevice(device: UsbDevice, ask: Boolean) {
        if (link != null) return
        val accessory = device.vendorId == GOOGLE && device.productId in ACCESSORY_PRODUCTS
        if (!accessory && !looksLikePhone(device)) return
        if (!manager.hasPermission(device)) {
            if (ask || !accessory) manager.requestPermission(device, permissionIntent())
            return
        }
        if (accessory) openAccessory(device) else thread(name = "usb-switch") { guarded { switchToAccessory(device) } }
    }

    /** A cable that misbehaves, or a device Android won't open, is logged and left alone. */
    private inline fun guarded(block: () -> Unit) {
        try {
            block()
        } catch (e: Exception) {
            AppLog.error("usb", e)
        }
    }

    private fun switchToAccessory(device: UsbDevice) {
        val connection = manager.openDevice(device) ?: return
        try {
            val version = ByteArray(2)
            val read = connection.controlTransfer(VENDOR_IN, AOA_GET_PROTOCOL, 0, 0, version, version.size, TIMEOUT_MS)
            val protocol = (version[0].toInt() and 0xFF) or ((version[1].toInt() and 0xFF) shl 8)
            AppLog.write("usb", "${device.productName}: accessory protocol ${if (read == 2) protocol else "not supported"}")
            if (read != 2 || protocol < 1) return
            IDENTITY.forEachIndexed { index, text ->
                val bytes = "$text\u0000".toByteArray()
                connection.controlTransfer(VENDOR_OUT, AOA_SEND_STRING, 0, index, bytes, bytes.size, TIMEOUT_MS)
            }
            peerName = device.productName
            connection.controlTransfer(VENDOR_OUT, AOA_START, 0, 0, null, 0, TIMEOUT_MS)
        } finally {
            connection.close()
        }
    }

    private fun openAccessory(device: UsbDevice) {
        val connection = manager.openDevice(device) ?: return
        val face = device.getInterface(0)
        val bulk = (0 until face.endpointCount).map(face::getEndpoint).filter { it.type == UsbConstants.USB_ENDPOINT_XFER_BULK }
        val input = bulk.firstOrNull { it.direction == UsbConstants.USB_DIR_IN }
        val output = bulk.firstOrNull { it.direction == UsbConstants.USB_DIR_OUT }
        if (input == null || output == null || !connection.claimInterface(face, true)) {
            connection.close()
            return
        }
        fun checked(n: Int) = if (n < 0) throw IOException("the USB cable was disconnected") else n
        start(
            Pump(
                key = device.deviceName,
                readBuffer = HOST_READ,
                read = { buffer, offset, length -> checked(connection.bulkTransfer(input, buffer, offset, length, 0)) },
                write = { buffer, offset, length -> checked(connection.bulkTransfer(output, buffer, offset, length, 0)) },
                release = {
                    connection.releaseInterface(face)
                    connection.close()
                },
            ),
            host = true,
            name = peerName ?: device.productName,
        )
    }

    private fun start(pump: Pump, host: Boolean, name: String?) {
        AppLog.write("usb", "connected as ${if (host) "host" else "accessory"} to ${name ?: "a phone"}")
        link = pump
        emit(PlatformEvent(PlatformEventKind.USB_CONNECTED, name = name, fd = pump.engineFd.toLong(), host = host))
        pump.begin()
    }

    private fun permissionIntent(): PendingIntent = PendingIntent.getBroadcast(
        context,
        0,
        Intent(PERMISSION).setPackage(context.packageName),
        PendingIntent.FLAG_MUTABLE,
    )

    /**
     * Copies between the cable and a socket pair, on two threads. The engine owns
     * the other end of the pair; either side closing ends the link.
     */
    private inner class Pump(
        val key: String,
        private val readBuffer: Int,
        private val read: Transfer,
        private val write: Transfer,
        private val release: () -> Unit,
    ) {
        private val closed = AtomicBoolean(false)
        private val socket: ParcelFileDescriptor
        val engineFd: Int

        init {
            val (ours, engines) = ParcelFileDescriptor.createSocketPair()
            socket = ours
            engineFd = engines.detachFd()
        }

        fun begin() {
            val toEngine = FileOutputStream(socket.fileDescriptor)
            val fromEngine = FileInputStream(socket.fileDescriptor)
            run("usb-in") {
                val buffer = ByteArray(readBuffer)
                while (true) {
                    val n = read(buffer, 0, buffer.size)
                    if (n < 0) break
                    toEngine.write(buffer, 0, n)
                }
            }
            run("usb-out") {
                val buffer = ByteArray(ENGINE_READ)
                while (true) {
                    val n = fromEngine.read(buffer)
                    if (n < 0) break
                    writeAll(write, buffer, n)
                }
            }
        }

        private fun run(name: String, body: () -> Unit) = thread(name = name) {
            try {
                body()
            } catch (e: Exception) {
                AppLog.write("usb", "$name stopped: $e")
            } finally {
                close()
            }
        }

        fun close() {
            if (!closed.compareAndSet(false, true)) return
            runCatching { Os.shutdown(socket.fileDescriptor, OsConstants.SHUT_RDWR) }
            runCatching { socket.close() }
            runCatching(release)
            if (link === this) link = null
            emit(PlatformEvent(PlatformEventKind.USB_DISCONNECTED))
        }
    }

    private companion object {
        const val PERMISSION = "app.nearvolt.USB_PERMISSION"
        const val ACCESSORY = "accessory"

        /** Manufacturer, model, description, version, URI and serial. Android shows the URI when the other phone lacks the app. */
        val IDENTITY = listOf("Nearvolt", "Nearvolt", "Nearvolt file transfer", "1", "https://github.com/vishalpa8/Nearvolt/releases/latest", "1")

        const val GOOGLE = 0x18D1
        /** Accessory, and accessory with ADB. */
        val ACCESSORY_PRODUCTS = 0x2D00..0x2D01
        const val AOA_GET_PROTOCOL = 51
        const val AOA_SEND_STRING = 52
        const val AOA_START = 53
        const val VENDOR_IN = UsbConstants.USB_DIR_IN or UsbConstants.USB_TYPE_VENDOR
        const val VENDOR_OUT = UsbConstants.USB_DIR_OUT or UsbConstants.USB_TYPE_VENDOR
        const val TIMEOUT_MS = 1000

        /** The accessory driver hands out at most 16 KiB per read. */
        const val ACCESSORY_READ = 16 * 1024
        const val HOST_READ = 64 * 1024

        /** Odd-sized, so full reads never land on a packet boundary. */
        const val ENGINE_READ = 64 * 1024 - 1

        /** Divides every USB bulk packet size: 64, 512 and 1024 bytes. */
        const val SMALLEST_PACKET = 64

        /** Phones show only these interface classes, or none when charging only. Keyboards, drives, hubs and audio are left alone. */
        val PHONE_CLASSES = setOf(
            UsbConstants.USB_CLASS_STILL_IMAGE,
            UsbConstants.USB_CLASS_VENDOR_SPEC,
            UsbConstants.USB_CLASS_COMM,
            UsbConstants.USB_CLASS_CDC_DATA,
            UsbConstants.USB_CLASS_WIRELESS_CONTROLLER,
        )

        fun looksLikePhone(device: UsbDevice) = device.deviceClass != UsbConstants.USB_CLASS_HUB &&
            (0 until device.interfaceCount).all { device.getInterface(it).interfaceClass in PHONE_CLASSES }

        /**
         * Writes so the last USB packet is short. A read on the other side then
         * returns right away instead of waiting for more data to fill its buffer.
         */
        fun writeAll(write: Transfer, buffer: ByteArray, length: Int) {
            val split = if (length % SMALLEST_PACKET == 0) length - 1 else length
            var done = 0
            while (done < split) done += write(buffer, done, split - done)
            if (split < length) write(buffer, split, 1)
        }
    }
}
