package app.nearvolt

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class AppLogTest {
    @Test
    fun `addresses on a home network keep only their first two parts`() {
        assertEquals("connected to 10.0.x.x on port 4000", AppLog.redact("connected to 10.0.3.7 on port 4000"))
    }

    @Test
    fun `a Wi-Fi Direct group address stays readable because it says how the phones linked`() {
        assertEquals("via 192.168.49.x", AppLog.redact("via 192.168.49.1"))
    }

    @Test
    fun `paths and file names are left out`() {
        val text = AppLog.redact("can't write /storage/emulated/0/Download/Nearvolt/holiday photo.jpg and IMG_1.png")
        assertFalse(text.contains("holiday"))
        assertFalse(text.contains("IMG_1"))
        assertFalse(text.contains("/storage"))
    }

    @Test
    fun `crash traces stay readable`() {
        val line = "at app.nearvolt.Ble.expire(Ble.kt:58)"
        assertEquals(line, AppLog.redact(line))
    }
}
