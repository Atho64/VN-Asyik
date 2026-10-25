package com.atho.vnasyik

import android.app.Activity
import android.content.Intent
import android.os.ParcelFileDescriptor
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.tauri.plugin.Invoke
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Runs only against the separately installed verification APK, never user data. */
@RunWith(AndroidJUnit4::class)
class AndroidTrackingTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val target = instrumentation.targetContext
    private val verificationPackage = "app.vntracker.verificationandroid"
    private val runnerPackage = "com.android.settings"
    private lateinit var activity: Activity
    private lateinit var webView: WebView

    @Test
    fun deniedAccessAndForegroundReplayThroughRustIpc() {
        assertEquals("This regression must use the isolated verification APK", verificationPackage, target.packageName)
        val previousMode = Regex("GET_USAGE_STATS: (\\w+)")
            .find(shell("appops get $verificationPackage GET_USAGE_STATS"))
            ?.groupValues?.get(1) ?: "default"
        val games = mutableListOf<String>()
        try {
            activity = instrumentation.startActivitySync(
                Intent(target, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            waitUntil("Tauri WebView readiness") {
                instrumentation.runOnMainSync {
                    findWebView(activity.window.decorView)?.let { webView = it }
                }
                ::webView.isInitialized && evaluate("Boolean(window.__TAURI_INTERNALS__?.invoke)") == "true"
            }
            val id = createGame("Foreground regression ${UUID.randomUUID()}")
            val otherId = createGame("Shared package regression ${UUID.randomUUID()}")
            games.addAll(listOf(id, otherId))
            for (game in games) {
                invoke("tracking_map_runner_target", JSONObject().put("input", JSONObject()
                    .put("gameId", game).put("runnerKind", "custom").put("package", runnerPackage)))
            }

            shell("appops set $verificationPackage GET_USAGE_STATS deny")
            assertFalse(invoke("tracking_usage_access").getBoolean("granted"))
            val denied = invokeResponse("launch_game", JSONObject().put("gameId", id))
            assertFalse("Play must fail without Usage Access", denied.getBoolean("ok"))
            assertTrue(denied.getJSONObject("error").getString("message").contains("Usage access"))
            assertEquals(0L, invoke("get_game", JSONObject().put("id", id)).getLong("tracked_ms"))
            val deniedNative = nativeQuery(System.currentTimeMillis() - 1_000, System.currentTimeMillis())
            assertFalse("Native query also enforces Usage Access", deniedNative.first)
            assertTrue(deniedNative.second.contains("usage access not granted"))

            shell("appops set $verificationPackage GET_USAGE_STATS allow")
            assertTrue(invoke("tracking_usage_access").getBoolean("granted"))
            val since = System.currentTimeMillis()
            invoke("launch_game", JSONObject().put("gameId", id))
            Thread.sleep(4_000)
            shell("input keyevent 3") // Other foreground package, without revealing its identity.
            Thread.sleep(4_000)
            shell("am start -W -a android.settings.SETTINGS")
            Thread.sleep(4_000)
            shell("am start -W -n $verificationPackage/com.atho.vnasyik.MainActivity")
            Thread.sleep(1_500)
            val until = System.currentTimeMillis()
            val (querySucceeded, raw) = nativeQuery(since, until)
            assertTrue("Usage event query must succeed on the device", querySucceeded)
            val events = JSONArray(raw)
            var runnerResumes = 0
            var selfResumes = 0
            for (index in 0 until events.length()) {
                val event = events.getJSONObject(index)
                val pkg = event.optString("package", "")
                assertTrue("Unrelated app identity crossed native IPC", pkg in listOf("", "null", runnerPackage, UsageEventPolicy.SELF_PACKAGE_MARKER))
                if (pkg != runnerPackage) assertTrue(event.isNull("activity"))
                if (event.getInt("kind") == 1 && pkg == runnerPackage) runnerResumes++
                if (event.getInt("kind") == 1 && pkg == UsageEventPolicy.SELF_PACKAGE_MARKER) selfResumes++
            }
            assertTrue("Both real runner foreground intervals must appear", runnerResumes >= 2)
            assertTrue("Returning to VN Asyik must appear in the usage log", selfResumes >= 1)
            val reconciled = invoke("tracking_android_session")
            assertFalse("A return ends explicit attribution", reconciled.getBoolean("active"))
            assertTrue(reconciled.isNull("reason"))
            val first = invoke("get_game", JSONObject().put("id", id)).getLong("tracked_ms")
            assertTrue("Real foreground time must be recorded", first >= 6_000)
            assertTrue("Home time must not count as runner playtime", first < until - since - 2_000)
            assertEquals(0L, invoke("get_game", JSONObject().put("id", otherId)).getLong("tracked_ms"))
            invoke("tracking_android_session")
            assertEquals("Replay must be idempotent", first, invoke("get_game", JSONObject().put("id", id)).getLong("tracked_ms"))
            println("ANDROID_FOREGROUND_VERIFIED countedMs=$first observedWindowMs=${until - since} runnerResumes=$runnerResumes selfResumes=$selfResumes")
        } finally {
            shell("appops set $verificationPackage GET_USAGE_STATS $previousMode")
            if (::webView.isInitialized) {
                for (id in games) {
                    runCatching { invokeResponse("delete_game", JSONObject().put("id", id)) }
                }
            }
        }
    }

    private fun createGame(title: String): String = invoke("create_game", JSONObject().put("input", JSONObject()
        .put("title", title).put("status", "playing"))).getJSONObject("meta").getString("id")

    private fun invoke(command: String, args: JSONObject = JSONObject()): JSONObject {
        val response = invokeResponse(command, args)
        assertTrue("IPC $command failed: $response", response.getBoolean("ok"))
        return response.getJSONObject("value")
    }

    // @tauri-apps/api/core.js invokes this same verified Tauri bridge signature.
    private fun invokeResponse(command: String, args: JSONObject = JSONObject()): JSONObject {
        val key = "__vnTrackingVerification"
        evaluate("window.$key = null; window.__TAURI_INTERNALS__.invoke(${JSONObject.quote(command)}, $args).then(value => window.$key = {ok:true,value}, error => window.$key = {ok:false,error}); null")
        var response: JSONObject? = null
        waitUntil("IPC $command") {
            val raw = evaluate("JSON.stringify(window.$key)")
            if (raw != "null") {
                val decoded = JSONTokener(raw).nextValue() as? String
                if (decoded != null && decoded != "null") response = JSONObject(decoded)
            }
            response != null
        }
        return response!!
    }

    // Constructor/response callbacks verified from Tauri 2.12.4 Invoke.kt.
    private fun nativeQuery(since: Long, until: Long): Pair<Boolean, String> {
        val latch = CountDownLatch(1)
        val result = AtomicReference<Pair<Boolean, String>>()
        val args = JSONObject().put("packages", JSONArray().put(runnerPackage)).put("sinceMs", since).put("untilMs", until)
        val response: (Long, String) -> Unit = { callback, data ->
            result.set(Pair(callback == 1L, data)); latch.countDown()
        }
        // ObjectMapper is a runtime implementation dependency of Tauri rather
        // than exported on the test compile classpath. Resolve that verified
        // constructor reflectively without introducing a new dependency.
        val mapper = Class.forName("com.fasterxml.jackson.databind.ObjectMapper")
            .getDeclaredConstructor().newInstance()
        val call = Invoke::class.java.constructors.single().newInstance(
            1L, "queryEvents", 1L, 2L, response, args.toString(), mapper,
        ) as Invoke
        instrumentation.runOnMainSync { UsageStatsPlugin(activity).queryEvents(call) }
        assertTrue("Native query timed out", latch.await(15, TimeUnit.SECONDS))
        return result.get()
    }

    private fun evaluate(script: String): String {
        val latch = CountDownLatch(1)
        val result = AtomicReference<String>()
        // Android WebView docs require evaluateJavascript on its UI thread.
        instrumentation.runOnMainSync {
            webView.evaluateJavascript(script) { result.set(it); latch.countDown() }
        }
        assertTrue("WebView evaluation timed out", latch.await(15, TimeUnit.SECONDS))
        return result.get()
    }

    private fun findWebView(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) for (index in 0 until view.childCount) {
            findWebView(view.getChildAt(index))?.let { return it }
        }
        return null
    }

    private fun waitUntil(label: String, condition: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (System.nanoTime() < deadline) {
            if (condition()) return
            Thread.sleep(100)
        }
        fail("Timed out waiting for $label")
    }

    private fun shell(command: String): String = ParcelFileDescriptor.AutoCloseInputStream(
        instrumentation.uiAutomation.executeShellCommand(command),
    ).bufferedReader().use { it.readText() }
}
