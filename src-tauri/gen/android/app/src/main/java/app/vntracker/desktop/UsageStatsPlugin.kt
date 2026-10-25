package app.vntracker.desktop

import android.app.Activity
import android.app.AppOpsManager
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Process
import android.provider.Settings
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
internal class UsageQueryArgs {
    lateinit var packages: Array<String>
    var sinceMs: Long = 0
    var untilMs: Long = 0
}

@InvokeArg
internal class PackageLaunchArgs {
    lateinit var `package`: String
}

/**
 * Foreground usage and standard package launching, checked against Android SDK
 * 37 UsageStatsManager, UsageEvents.Event, and PackageManager source/docs.
 * Play attribution is persisted in Rust; no elapsed-time estimate runs here.
 */
@TauriPlugin
class UsageStatsPlugin(private val activity: Activity) : Plugin(activity) {

    /**
     * Whether the user has granted usage access.
     */
    private fun hasUsageAccess(): Boolean {
        val appOps = activity.getSystemService(Context.APP_OPS_SERVICE) as AppOpsManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            appOps.unsafeCheckOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), activity.packageName,
            )
        } else {
            @Suppress("DEPRECATION")
            appOps.checkOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), activity.packageName,
            )
        }
        return if (mode == AppOpsManager.MODE_DEFAULT) {
            activity.checkCallingOrSelfPermission(
                android.Manifest.permission.PACKAGE_USAGE_STATS,
            ) == android.content.pm.PackageManager.PERMISSION_GRANTED
        } else {
            mode == AppOpsManager.MODE_ALLOWED
        }
    }

    @Command
    fun status(invoke: Invoke) {
        val result = JSObject()
        result.put("granted", hasUsageAccess())
        result.put("settingsAction", Settings.ACTION_USAGE_ACCESS_SETTINGS)
        invoke.resolve(result)
    }

    @Command
    fun openSettings(invoke: Invoke) {
        try {
            val intent = Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS)
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "could not open usage access settings")
        }
    }

    /** PackageManager's public launcher contract supports forks without guessing IDs. */
    @Command
    fun installedApps(invoke: Invoke) {
        try {
            val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
            @Suppress("DEPRECATION")
            val matches = activity.packageManager.queryIntentActivities(intent, 0)
            val apps = matches
                .filter { it.activityInfo.exported && it.activityInfo.enabled }
                .filter { it.activityInfo.packageName != activity.packageName }
                .distinctBy { it.activityInfo.packageName }
                .map {
                    mapOf(
                        "package" to it.activityInfo.packageName,
                        "label" to it.loadLabel(activity.packageManager).toString(),
                    )
                }
                .sortedBy { it["label"]?.lowercase() }
            invoke.resolveObject(apps)
        } catch (_: Exception) {
            invoke.reject("could not list installed launcher apps")
        }
    }

    /** The documented PackageManager launcher contract opens the selected app. */
    @Command
    fun launchPackage(invoke: Invoke) {
        val args = invoke.parseArgs(PackageLaunchArgs::class.java)
        val packageName = args.`package`.trim()
        if (!hasUsageAccess()) {
            invoke.reject("usage access not granted")
            return
        }
        if (packageName == activity.packageName) {
            invoke.reject("select a game or runner, not VN Asyik itself")
            return
        }
        activity.runOnUiThread {
            try {
                val intent = activity.packageManager.getLaunchIntentForPackage(packageName)
                if (intent == null) {
                    invoke.reject("the mapped package is not installed or has no launcher activity")
                    return@runOnUiThread
                }
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                activity.startActivity(intent)
                invoke.resolve()
            } catch (_: Exception) {
                invoke.reject("Android refused to open the mapped package")
            }
        }
    }

    @Command
    fun queryEvents(invoke: Invoke) {
        val args = invoke.parseArgs(UsageQueryArgs::class.java)

        if (!hasUsageAccess()) {
            invoke.reject("usage access not granted")
            return
        }

        if (args.packages.isEmpty() || args.sinceMs < 0L || args.untilMs <= args.sinceMs) {
            invoke.reject("invalid usage query window")
            return
        }
        try {
            val manager = activity.getSystemService(Context.USAGE_STATS_SERVICE) as UsageStatsManager
            // SDK: beginning inclusive, end exclusive; null while user storage is locked.
            val events = manager.queryEvents(args.sinceMs, args.untilMs)
            if (events == null) {
                invoke.reject("usage events unavailable (device locked or not unlocked)")
                return
            }
            val wanted = args.packages.toHashSet()
            val out = ArrayList<Map<String, Any?>>()
            val event = UsageEvents.Event()
            while (events.hasNextEvent()) {
                events.getNextEvent(event)
                UsageEventPolicy.payload(
                    wanted, activity.packageName, event.packageName,
                    event.className, event.eventType, event.timeStamp,
                )?.let(out::add)
            }
            invoke.resolveObject(out)
        } catch (_: SecurityException) {
            invoke.reject("usage access not granted")
        } catch (_: Exception) {
            invoke.reject("usage events unavailable")
        }
    }
}
