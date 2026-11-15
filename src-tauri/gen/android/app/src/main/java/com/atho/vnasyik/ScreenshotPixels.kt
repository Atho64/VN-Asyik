package com.atho.vnasyik

import java.nio.ByteBuffer
import kotlin.math.max

internal fun boundedCaptureSize(width: Int, height: Int): Pair<Int, Int> {
    require(width > 0 && height > 0)
    val scale = 1920.0 / max(width, height).coerceAtLeast(1920)
    return max(1, (width * scale).toInt()) to max(1, (height * scale).toInt())
}

/** ImageReader's last row need not include its padding. Copy actual pixels only. */
internal fun packedRgba(buffer: ByteBuffer, width: Int, height: Int, rowStride: Int, pixelStride: Int): ByteBuffer {
    require(width in 1..1920 && height in 1..1920 && pixelStride >= 4)
    val rowBytes = (width - 1).toLong() * pixelStride + 4
    require(rowStride >= rowBytes && (height - 1).toLong() * rowStride + rowBytes <= buffer.remaining())
    val packed = ByteBuffer.allocate(width * height * 4)
    val origin = buffer.position()
    for (row in 0 until height) {
        for (column in 0 until width) {
            val offset = origin + row * rowStride + column * pixelStride
            for (component in 0..3) packed.put(buffer.get(offset + component))
        }
    }
    packed.flip()
    return packed
}
