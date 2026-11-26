package app.nearvolt

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.cardemulation.HostApduService
import android.nfc.tech.IsoDep
import android.os.Bundle
import java.io.ByteArrayOutputStream

/**
 * Tap to pair. The receiver acts as an NFC card that serves its private ticket;
 * the sender reads it in reader mode while the send page is open.
 */
class Nfc(private val activity: Activity, private val emit: (PlatformEvent) -> Unit) {
    private val adapter: NfcAdapter? = NfcAdapter.getDefaultAdapter(activity)
    private var reading = false

    fun serve(ticket: String) {
        TicketCardService.ticket = ticket.toByteArray()
    }

    fun stopServing() {
        TicketCardService.ticket = null
    }

    fun read() {
        reading = true
        val flags = NfcAdapter.FLAG_READER_NFC_A or NfcAdapter.FLAG_READER_NFC_B or
            NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK or NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS
        runCatching { adapter?.enableReaderMode(activity, ::onTag, flags, null) }
    }

    fun stopReading() {
        reading = false
        runCatching { adapter?.disableReaderMode(activity) }
    }

    /** Reader mode ends when the activity pauses, so it is restored on resume. */
    fun onResume() {
        if (reading) read()
    }

    private fun onTag(tag: Tag) {
        val iso = IsoDep.get(tag) ?: return
        runCatching {
            iso.use {
                it.connect()
                it.timeout = 2_000
                if (!ok(it.transceive(SELECT))) return
                val ticket = ByteArrayOutputStream()
                while (true) {
                    val offset = ticket.size()
                    val response = it.transceive(byteArrayOf(0x00, 0xB0.toByte(), (offset shr 8).toByte(), offset.toByte(), 0x00))
                    if (!ok(response) || response.size == 2) break
                    ticket.write(response, 0, response.size - 2)
                }
                if (ticket.size() > 0) {
                    emit(PlatformEvent(PlatformEventKind.TICKET_TAPPED, ticket = ticket.toString(Charsets.UTF_8.name())))
                }
            }
        }
    }

    private fun ok(response: ByteArray) =
        response.size >= 2 && response[response.size - 2] == 0x90.toByte() && response[response.size - 1] == 0x00.toByte()

    companion object {
        /** Proprietary application ID: F0 followed by "NEARVOLT". */
        val AID = byteArrayOf(0xF0.toByte()) + "NEARVOLT".toByteArray()
        val SELECT = byteArrayOf(0x00, 0xA4.toByte(), 0x04, 0x00, AID.size.toByte()) + AID + byteArrayOf(0x00)
    }
}

/** Serves the ticket in chunks: SELECT, then READ BINARY with the offset in P1 and P2. */
class TicketCardService : HostApduService() {
    override fun processCommandApdu(apdu: ByteArray, extras: Bundle?): ByteArray {
        val ticket = ticket ?: return NOT_FOUND
        return when {
            apdu.size >= 4 && apdu[1] == 0xA4.toByte() -> OK
            apdu.size >= 4 && apdu[1] == 0xB0.toByte() -> {
                val offset = ((apdu[2].toInt() and 0xFF) shl 8) or (apdu[3].toInt() and 0xFF)
                val start = offset.coerceAtMost(ticket.size)
                ticket.copyOfRange(start, (start + CHUNK).coerceAtMost(ticket.size)) + OK
            }
            else -> UNKNOWN
        }
    }

    override fun onDeactivated(reason: Int) {}

    companion object {
        @Volatile
        var ticket: ByteArray? = null
        private const val CHUNK = 240
        private val OK = byteArrayOf(0x90.toByte(), 0x00)
        private val NOT_FOUND = byteArrayOf(0x6A, 0x82.toByte())
        private val UNKNOWN = byteArrayOf(0x6D, 0x00)
    }
}
