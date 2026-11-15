package com.atho.vnasyik

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Build
import androidx.activity.result.ActivityResult
import androidx.core.app.NotificationManagerCompat
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = "notifications")])
class ScreenshotPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun capture(invoke: Invoke) {
        if (ScreenshotCaptureService.isPending()) {
            finish(invoke, "screenshot_busy")
            return
        }
        if (Build.VERSION.SDK_INT >= 33 && getPermissionState("notifications") != PermissionState.GRANTED) {
            requestPermissionForAlias("notifications", invoke, "notificationResult")
            return
        }
        requestProjection(invoke)
    }

    @PermissionCallback
    fun notificationResult(invoke: Invoke) {
        if (getPermissionState("notifications") != PermissionState.GRANTED) {
            finish(invoke, "screenshot_notifications")
        } else {
            requestProjection(invoke)
        }
    }

    private fun requestProjection(invoke: Invoke) {
        if (!NotificationManagerCompat.from(activity).areNotificationsEnabled()) {
            finish(invoke, "screenshot_notifications")
            return
        }
        if (!ScreenshotCaptureService.reserve { dataUrl, error ->
                invoke.resolve(JSObject().put("dataUrl", dataUrl ?: org.json.JSONObject.NULL)
                    .put("error", error ?: org.json.JSONObject.NULL))
            }) {
            finish(invoke, "screenshot_busy")
            return
        }
        try {
            val manager = activity.getSystemService(MediaProjectionManager::class.java)
            // Default intent retains Android 14+'s user choice of a single app.
            startActivityForResult(invoke, manager.createScreenCaptureIntent(), "projectionResult")
        } catch (_: Exception) {
            ScreenshotCaptureService.complete(null, "screenshot_failed")
        }
    }

    @ActivityCallback
    fun projectionResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK || result.data == null) {
            ScreenshotCaptureService.complete(null, "screenshot_cancelled")
            return
        }
        try {
            val intent = Intent(activity, ScreenshotCaptureService::class.java).apply {
                action = ScreenshotCaptureService.START
                putExtra("consent", result.data)
                putExtra("resultCode", result.resultCode)
            }
            if (Build.VERSION.SDK_INT >= 26) activity.startForegroundService(intent)
            else activity.startService(intent)
        } catch (_: Exception) {
            ScreenshotCaptureService.complete(null, "screenshot_failed")
        }
    }

    private fun finish(invoke: Invoke, error: String) {
        invoke.resolve(JSObject().put("dataUrl", org.json.JSONObject.NULL).put("error", error))
    }
}
