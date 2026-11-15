package com.atho.vnasyik

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.Image
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.view.WindowManager
import java.io.ByteArrayOutputStream
import java.util.UUID

/** A fresh OS consent token produces at most one stored screenshot. */
class ScreenshotCaptureService : Service() {
    companion object {
        const val START = "com.atho.vnasyik.SCREENSHOT_START"
        private const val CAPTURE = "com.atho.vnasyik.SCREENSHOT_CAPTURE"
        private const val CANCEL = "com.atho.vnasyik.SCREENSHOT_CANCEL"
        private const val CHANNEL = "vn_asyik_screenshots"
        private const val NOTIFICATION = 2027
        private var pending: ((String?, String?) -> Unit)? = null

        @Synchronized fun isPending(): Boolean = pending != null

        @Synchronized fun reserve(callback: (String?, String?) -> Unit): Boolean {
            if (pending != null) return false
            pending = callback
            return true
        }

        fun complete(dataUrl: String?, error: String?) {
            val callback = synchronized(this) { pending.also { pending = null } }
            callback?.invoke(dataUrl, error)
        }
    }

    private val main = Handler(Looper.getMainLooper())
    private lateinit var thread: HandlerThread
    private lateinit var worker: Handler
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var reader: ImageReader? = null
    private var latest: Image? = null
    private var finished = false
    private var captureAfter: Long? = null
    private var contentVisible = true
    private val session = UUID.randomUUID().toString()
    private val timeout = Runnable { worker.post { finish(null, "screenshot_timeout") } }
    private val frameTimeout = Runnable { finish(null, "screenshot_timeout") }

    override fun onCreate() {
        super.onCreate()
        thread = HandlerThread("vn-asyik-screenshot").apply { start() }
        worker = Handler(thread.looper)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == START) {
            if (!isPending() || projection != null) return START_NOT_STICKY
            try {
                val manager = getSystemService(NotificationManager::class.java)
                if (Build.VERSION.SDK_INT >= 26) {
                    manager.createNotificationChannel(NotificationChannel(CHANNEL,
                        getString(R.string.screenshot_channel), NotificationManager.IMPORTANCE_LOW))
                    check(manager.getNotificationChannel(CHANNEL).importance != NotificationManager.IMPORTANCE_NONE)
                }
                val notification = notification()
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
                } else startForeground(NOTIFICATION, notification)
                val consent = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra("consent", Intent::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra<Intent>("consent")
                }
                checkNotNull(consent)
                worker.post { startProjection(intent.getIntExtra("resultCode", 0), consent) }
                main.postDelayed(timeout, 120_000)
            } catch (_: Exception) {
                worker.post { finish(null, "screenshot_failed") }
            }
        } else if (intent?.getStringExtra("session") == session) {
            when (intent.action) {
                CAPTURE -> worker.post {
                    if (!finished && captureAfter == null) {
                        // Capture a new frame after the notification drawer closes.
                        captureAfter = SystemClock.uptimeMillis() + 3_000
                        worker.postDelayed({ saveLatest() }, 3_000)
                        worker.postDelayed(frameTimeout, 10_000)
                    }
                }
                CANCEL -> worker.post { finish(null, "screenshot_cancelled") }
            }
        }
        return START_NOT_STICKY
    }

    private fun startProjection(resultCode: Int, consent: Intent) {
        if (finished) return
        try {
            projection = getSystemService(MediaProjectionManager::class.java)
                .getMediaProjection(resultCode, consent)
            projection!!.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() { finish(null, "screenshot_cancelled") }
                override fun onCapturedContentVisibilityChanged(isVisible: Boolean) {
                    contentVisible = isVisible
                    if (!isVisible) {
                        latest?.close()
                        latest = null
                    }
                }
                override fun onCapturedContentResize(width: Int, height: Int) {
                    if (!finished && width > 0 && height > 0) resize(width, height)
                }
            }, worker)
            val windowManager = getSystemService(WindowManager::class.java)
            val size = if (Build.VERSION.SDK_INT >= 30) {
                windowManager.maximumWindowMetrics.bounds.let { it.width() to it.height() }
            } else {
                val metrics = android.util.DisplayMetrics()
                @Suppress("DEPRECATION")
                windowManager.defaultDisplay.getRealMetrics(metrics)
                metrics.widthPixels to metrics.heightPixels
            }
            val bounds = boundedCaptureSize(size.first, size.second)
            reader = newReader(bounds.first, bounds.second)
            display = projection!!.createVirtualDisplay("VN Asyik screenshot", bounds.first, bounds.second,
                resources.configuration.densityDpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                reader!!.surface, null, worker)
        } catch (_: Exception) {
            finish(null, "screenshot_failed")
        }
    }

    private fun resize(width: Int, height: Int) {
        if (display == null) return
        try {
            val bounds = boundedCaptureSize(width, height)
            val previous = reader
            latest?.close()
            latest = null
            val next = newReader(bounds.first, bounds.second)
            reader = next
            try {
                display!!.surface = next.surface
                display!!.resize(bounds.first, bounds.second, resources.configuration.densityDpi)
            } finally {
                previous?.close()
            }
        } catch (_: Exception) { finish(null, "screenshot_failed") }
    }

    private fun newReader(width: Int, height: Int): ImageReader =
        ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 3).apply {
            setOnImageAvailableListener({ available ->
                if (available !== reader || finished) return@setOnImageAvailableListener
                val image = try { available.acquireLatestImage() } catch (_: Exception) { null }
                if (image != null && contentVisible) {
                    latest?.close()
                    latest = image
                    saveLatest()
                } else {
                    image?.close()
                }
            }, worker)
        }

    private fun saveLatest() {
        val after = captureAfter ?: return
        if (finished || !contentVisible || SystemClock.uptimeMillis() < after) return
        val image = latest ?: return
        latest = null
        image.use {
            try {
                val plane = image.planes[0]
                val bytes = packedRgba(plane.buffer, image.width, image.height, plane.rowStride, plane.pixelStride)
                val bitmap = Bitmap.createBitmap(image.width, image.height, Bitmap.Config.ARGB_8888)
                try {
                    bitmap.copyPixelsFromBuffer(bytes)
                    val encoded = ByteArrayOutputStream()
                    check(bitmap.compress(Bitmap.CompressFormat.JPEG, 88, encoded))
                    finish("data:image/jpeg;base64," + Base64.encodeToString(encoded.toByteArray(), Base64.NO_WRAP), null)
                } finally { bitmap.recycle() }
            } catch (_: Exception) { finish(null, "screenshot_encode") }
        }
    }

    private fun notification(): Notification {
        fun actionIntent(action: String, code: Int): PendingIntent = PendingIntent.getService(this, code,
            Intent(this, ScreenshotCaptureService::class.java).setAction(action).putExtra("session", session),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val builder = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL)
            else @Suppress("DEPRECATION") Notification.Builder(this)
        return builder.setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(getString(R.string.screenshot_title))
            .setContentText(getString(R.string.screenshot_notification_hint))
            .setStyle(Notification.BigTextStyle().bigText(getString(R.string.screenshot_notification_hint)))
            .setOngoing(true).setOnlyAlertOnce(true)
            .addAction(Notification.Action.Builder(null, getString(R.string.screenshot_capture), actionIntent(CAPTURE, 1)).build())
            .addAction(Notification.Action.Builder(null, getString(R.string.screenshot_cancel), actionIntent(CANCEL, 2)).build())
            .build()
    }

    private fun finish(dataUrl: String?, error: String?) {
        if (finished) return
        finished = true
        main.removeCallbacks(timeout)
        worker.removeCallbacks(frameTimeout)
        runCatching { display?.release() }
        display = null
        runCatching { latest?.close() }
        latest = null
        runCatching { reader?.close() }
        reader = null
        runCatching { projection?.stop() }
        projection = null
        main.post {
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            complete(dataUrl, error)
        }
    }

    override fun onDestroy() {
        main.removeCallbacks(timeout)
        worker.post {
            finish(null, "screenshot_cancelled")
            thread.quitSafely()
        }
        super.onDestroy()
    }
}
