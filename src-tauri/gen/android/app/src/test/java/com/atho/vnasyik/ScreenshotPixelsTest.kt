package com.atho.vnasyik

import java.nio.ByteBuffer
import org.junit.Assert.*
import org.junit.Test

class ScreenshotPixelsTest {
    @Test fun stripsRowPaddingWithoutRequiringPaddingOnLastRow() {
        val input = ByteBuffer.wrap(byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16))
        val output = packedRgba(input, 2, 2, 12, 4)
        val bytes = ByteArray(output.remaining()).also { output.get(it) }
        assertArrayEquals(ByteArray(16) { (it + 1).toByte() }, bytes)
    }

    @Test fun accountsForBufferPositionAndPixelStride() {
        val input = ByteBuffer.wrap(byteArrayOf(99, 1, 2, 3, 4, 88, 5, 6, 7, 8))
        input.position(1)
        val output = packedRgba(input, 2, 1, 10, 5)
        val bytes = ByteArray(output.remaining()).also { output.get(it) }
        assertArrayEquals(ByteArray(8) { (it + 1).toByte() }, bytes)
    }

    @Test fun rejectsTruncatedBuffersAndBoundsDimensions() {
        assertThrows(IllegalArgumentException::class.java) { packedRgba(ByteBuffer.allocate(7), 2, 1, 8, 4) }
        assertThrows(IllegalArgumentException::class.java) { boundedCaptureSize(0, 100) }
        assertEquals(1920 to 1080, boundedCaptureSize(3840, 2160))
        assertEquals(1080 to 1920, boundedCaptureSize(2160, 3840))
        assertEquals(640 to 480, boundedCaptureSize(640, 480))
    }
}
