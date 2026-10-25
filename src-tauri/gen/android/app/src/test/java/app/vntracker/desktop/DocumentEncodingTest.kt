package app.vntracker.desktop

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import java.nio.charset.CharacterCodingException

class DocumentEncodingTest {
    @Test
    fun validMultilingualJsonIsPreservedExactly() {
        val json = """{"title":"うみねこのなく頃に","notes":"Asyik membaca 📖"}"""
        assertEquals(json, decodeDocumentUtf8(json.toByteArray(Charsets.UTF_8)))
    }

    @Test
    fun malformedBytesInsideValidJsonAreRejectedBeforeImport() {
        // Lossy decoding makes this valid JSON with a silently altered title.
        val bytes = "{\"title\":\"".toByteArray(Charsets.UTF_8) +
            byteArrayOf(0xC3.toByte(), 0x28) + "\"}".toByteArray(Charsets.UTF_8)
        try {
            decodeDocumentUtf8(bytes)
            fail("Malformed UTF-8 must be rejected instead of replacing stored text")
        } catch (_: CharacterCodingException) { }
    }
}
