package com.atho.vnasyik

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Foreground service ensuring the Discord Gateway WebSocket remains alive and
 * connected while visual novels or emulators run in the foreground.
 */
class PresenceForegroundService : Service() {

    companion object {
        const val ACTION_START = "com.atho.vnasyik.PRESENCE_START"
        const val ACTION_UPDATE = "com.atho.vnasyik.PRESENCE_UPDATE"
        const val ACTION_STOP = "com.atho.vnasyik.PRESENCE_STOP"
        const val EXTRA_TITLE = "title"
        const val EXTRA_TEXT = "text"
        const val CHANNEL_ID = "vn_asyik_discord_rpc"
        const val NOTIFICATION_ID = 2026

        fun start(context: Context, title: String, text: String) {
            val intent = Intent(context, PresenceForegroundService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_TITLE, title)
                putExtra(EXTRA_TEXT, text)
            }
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
            } catch (e: Exception) {
                // If background launch is denied, don't crash the app
            }
        }

        fun update(context: Context, title: String, text: String) {
            start(context, title, text)
        }

        fun stop(context: Context) {
            try {
                context.stopService(Intent(context, PresenceForegroundService::class.java))
            } catch (e: Exception) {
                // Ignore service stop error
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onDestroy() {
        try {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } catch (_: Exception) {
        }
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_UPDATE -> {
                val title = intent.getStringExtra(EXTRA_TITLE) ?: "VN Asyik"
                val text = intent.getStringExtra(EXTRA_TEXT) ?: "Discord Rich Presence Active"
                val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                manager.notify(NOTIFICATION_ID, buildNotification(title, text))
                return START_STICKY
            }
            ACTION_START, null -> {
                val title = intent?.getStringExtra(EXTRA_TITLE) ?: "VN Asyik"
                val text = intent?.getStringExtra(EXTRA_TEXT) ?: "Discord Rich Presence Active"
                val notification = buildNotification(title, text)
                try {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        startForeground(
                            NOTIFICATION_ID,
                            notification,
                            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
                        )
                    } else {
                        startForeground(NOTIFICATION_ID, notification)
                    }
                } catch (e: Exception) {
                    // Fallback if foreground service cannot start with specialUse
                    stopSelf()
                    return START_NOT_STICKY
                }
                return START_STICKY
            }
            else -> return START_NOT_STICKY
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Discord Rich Presence",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Maintains Discord Rich Presence while running visual novels"
                setShowBadge(false)
            }
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(title: String, text: String): Notification {
        val launchIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pendingIntent = PendingIntent.getActivity(
            this,
            0,
            launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        return builder
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .build()
    }
}
