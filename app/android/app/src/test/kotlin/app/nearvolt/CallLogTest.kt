package app.nearvolt

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class CallLogTest {
    @Test
    fun `text is counted, never written, because it can be a code or a name`() {
        val shown = CallLog.describe("NV1SECRETCODE")
        assertEquals("text(13)", shown)
        assertFalse(shown.contains("SECRET"))
    }

    @Test
    fun `plain values and lists are described briefly`() {
        assertEquals("true", CallLog.describe(true))
        assertEquals("42", CallLog.describe(42))
        assertEquals("null", CallLog.describe(null))
        assertEquals("3 items", CallLog.describe(listOf(1, 2, 3)))
        assertEquals("done", CallLog.describe(Unit))
    }

    @Test
    fun `other results are named by kind so file lists stay out of the log`() {
        assertEquals("PickedFile", CallLog.describe(PickedFile("holiday.jpg", 1, null)))
    }
}
