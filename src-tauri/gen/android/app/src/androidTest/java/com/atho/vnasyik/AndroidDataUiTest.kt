package com.atho.vnasyik

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Real React -> Rust -> native document callback flow, with isolated synthetic files. */
@RunWith(AndroidJUnit4::class)
class AndroidDataUiTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val target = instrumentation.targetContext
    private lateinit var activity: Activity
    private lateinit var webView: WebView

    @Test
    fun unifiedDataControlsAndReviewedNativeImports() {
        assertEquals("Never test data imports against the normal application",
            "app.vntracker.verificationandroid", target.packageName)
        activity = instrumentation.startActivitySync(
            Intent(target, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        waitUntil("Tauri WebView") {
            instrumentation.runOnMainSync {
                findWebView(activity.window.decorView)?.let { webView = it }
            }
            ::webView.isInitialized && evaluate("Boolean(window.__TAURI_INTERNALS__?.invoke)") == "true"
        }
        val previousPrefs = invoke("get_prefs") as JSONObject
        val previousPath = decoded(evaluate("location.pathname"))
        val gameId = UUID.randomUUID().toString()
        val title = "Synthetic data UI 日本語 😀 $gameId"
        val directory = File(target.cacheDir, "data-ui-${UUID.randomUUID()}")
        assertTrue(directory.mkdirs())
        try {
            for (language in listOf("en", "id")) for (theme in listOf("dark", "light")) {
                openSettings(theme, language)
                assertControls(theme, language)
            }

            openSettings("dark", "en")
            val saved = File(directory, "export.json")
            run {
                val intent = selectedDocument(Intent.ACTION_CREATE_DOCUMENT, saved) {
                    evaluate("document.querySelectorAll('.data-actions button')[0].click(); null")
                    waitForIdle()
                }
                val exported = JSONObject(saved.readText(Charsets.UTF_8))
                assertEquals("application/json", intent.type)
                assertTrue(intent.hasCategory(Intent.CATEGORY_OPENABLE))
                assertTrue(intent.getStringExtra(Intent.EXTRA_TITLE)!!.startsWith("vn-asyik-library-"))
                assertEquals("vn-asyik-library", exported.getString("format"))
                assertEquals(1, exported.getInt("version"))
                assertTrue(exported.has("payload") && exported.has("prefs"))
                assertFalse(exported.has("header"))
            }

            // Use the real export header/current preferences, but only one new fixture row.
            val backup = invoke("export_backup") as JSONObject
            val header = backup.getJSONObject("header")
            val device = (invoke("sync_status") as JSONObject).getString("deviceId")
            val now = System.currentTimeMillis()
            val game = JSONObject().put("meta", JSONObject()
                .put("id", gameId).put("updated_at", now)
                .put("device_id_device", device).put("deleted", false))
                .put("title", title).put("status", "wishlist").put("manual_playtime_ms", 0)
            for (name in listOf("title_original", "vndb_id", "rating", "notes", "length_minutes",
                "accent_color", "cover_url", "cover_sexual", "cover_violence", "started_at", "finished_at")) {
                game.put(name, JSONObject.NULL)
            }
            backup.put("games", JSONArray().put(game)).put("chapters", JSONArray())
                .put("sessions", JSONArray()).put("endings", JSONArray())
            val input = File(directory, "synthetic-backup.json")
            input.writeText(backup.toString(), Charsets.UTF_8)
            assertNull(invoke("get_game", JSONObject().put("id", gameId)))
            val countBefore = (invoke("list_games") as JSONArray).length()

            // Old backups are accepted with the same default merge action.
            selectedDocument(Intent.ACTION_OPEN_DOCUMENT, input) { clickImport(); waitForPreview("Preview import") }
            assertPreviewLayout()
            assertNull("Preview must not write a game", invoke("get_game", JSONObject().put("id", gameId)))
            assertTrue(decoded(evaluate("document.querySelector('.data-preview-hint').textContent"))
                .contains("newer"))
            assertEquals("Merge data", decoded(evaluate("document.querySelector('.sheet__footer .btn--primary').textContent")))
            closePreview()
            assertNull("Cancelling must not write a game", invoke("get_game", JSONObject().put("id", gameId)))

            selectedDocument(Intent.ACTION_OPEN_DOCUMENT, input) { clickImport(); waitForPreview("Preview import") }
            // Applying must use the reviewed Rust snapshot, not reopen the now changed file.
            input.writeText("{}", Charsets.UTF_8)
            evaluate("document.querySelector('.sheet__footer .btn--primary').click(); null")
            waitForIdle()
            waitUntil("confirmed fixture imported") {
                (invoke("get_game", JSONObject().put("id", gameId)) as? JSONObject)
                    ?.getJSONObject("game")?.getString("title") == title
            }
            assertEquals(countBefore + 1, (invoke("list_games") as JSONArray).length())
            assertEquals(backup.getJSONObject("prefs").toString(), (invoke("get_prefs") as JSONObject).toString())
            assertEquals("Preview must close after applying", "false", evaluate("Boolean(document.querySelector('[role=dialog]'))"))

            // The new library envelope also supports an explicitly selected restore.
            val unified = JSONObject(saved.readText(Charsets.UTF_8))
            unified.getJSONObject("payload").put("games", JSONArray().put(game))
                .put("chapters", JSONArray()).put("sessions", JSONArray()).put("endings", JSONArray())
            unified.getJSONObject("prefs").put("theme", "light")
            input.writeText(unified.toString(), Charsets.UTF_8)
            selectedDocument(Intent.ACTION_OPEN_DOCUMENT, input) { clickImport(); waitForPreview("Preview import") }
            evaluate("document.querySelector('.data-advanced summary').click(); document.querySelector('.data-restore-option input').click(); null")
            waitUntil("explicit restore selected") {
                decoded(evaluate("document.querySelector('.sheet__footer .btn--primary').textContent")) == "Restore backup"
            }
            assertPreviewLayout()
            evaluate("document.querySelector('.sheet__footer .btn--primary').click(); null")
            waitUntil("restore applies preferences") {
                (invoke("get_prefs") as JSONObject).getString("theme") == "light"
            }
            waitUntil("restore preview closed") {
                evaluate("!document.querySelector('[role=dialog]')") == "true"
            }
            waitForIdle()

            val sameDevice = JSONObject().put("schema_version", header.getInt("schema_version"))
                .put("device_id", device).put("written_at_ms", now).put("generation", 0)
                .put("games", JSONArray()).put("chapters", JSONArray())
                .put("sessions", JSONArray()).put("endings", JSONArray())
            input.writeText(sameDevice.toString(), Charsets.UTF_8)
            selectedDocument(Intent.ACTION_OPEN_DOCUMENT, input) { clickImport(); waitForPreview("Preview import") }
            assertEquals("Same-device manual import permits merge", "false",
                evaluate("document.querySelector('.sheet__footer .btn--primary').disabled"))
            assertEquals("Legacy transfer lacks restore preferences", "false", evaluate("Boolean(document.querySelector('.data-advanced'))"))
            closePreview()

            input.writeText("{", Charsets.UTF_8)
            selectedDocument(Intent.ACTION_OPEN_DOCUMENT, input) {
                clickImport()
                waitUntil("malformed file error") {
                    evaluate("Boolean(document.querySelector('.data-screen [role=alert]'))") == "true"
                }
                waitForIdle()
            }
            assertEquals("false", evaluate("Boolean(document.querySelector('[role=dialog]'))"))
            assertEquals(countBefore + 1, (invoke("list_games") as JSONArray).length())
            println("ANDROID_DATA_UI_VERIFIED unifiedExport=true reviewedContentUri=true previewReadOnly=true cancelReadOnly=true immutableApply=true explicitRestore=true sameDeviceMerge=true malformedRejected=true")
        } finally {
            // Only this test's UUID is changed, inside the explicitly guarded disposable app.
            if (invoke("get_game", JSONObject().put("id", gameId)) != null) {
                invoke("delete_game", JSONObject().put("id", gameId))
            }
            invoke("set_theme", JSONObject().put("value", previousPrefs.getString("theme")))
            invoke("set_language", JSONObject().put("value", previousPrefs.getString("language")))
            invoke("set_nsfw_mode", JSONObject().put("value", previousPrefs.getString("nsfw")))
            invoke("set_reduce_effects", JSONObject().put("value", previousPrefs.getBoolean("reduce_effects")))
            evaluate("document.querySelector('a[href=' + ${JSONObject.quote(JSONObject.quote(previousPath))} + ']')?.click(); null")
            directory.listFiles()?.forEach { it.delete() }
            directory.delete()
        }
    }

    private fun openSettings(theme: String, language: String) {
        // Follow React navigation and preference controls. A hard navigation
        // during WebView.evaluateJavascript can discard its pending callback.
        waitUntil("React settings link") {
            evaluate("Boolean(document.querySelector('a[href=\"/settings\"]'))") == "true"
        }
        evaluate("document.querySelector('a[href=\"/settings\"]').click(); null")
        waitUntil("settings navigation") {
            evaluate("Boolean(document.querySelector('.settings > [role=tablist]'))") == "true"
        }
        evaluate("document.querySelectorAll('.settings > [role=tablist] button')[0].click(); null")
        waitUntil("general settings") {
            evaluate("Boolean(document.querySelector('.settings__group .settings__row'))") == "true"
        }
        evaluate("document.querySelectorAll('.settings__group .settings__row')[0].querySelectorAll('button')[${if (theme == "dark") 0 else 1}].click(); null")
        evaluate("document.querySelectorAll('.settings__group .settings__row')[2].querySelectorAll('button')[${if (language == "en") 0 else 1}].click(); null")
        waitUntil("settings in $theme/$language") {
            evaluate("document.documentElement.dataset.theme === ${JSONObject.quote(theme)} && document.documentElement.lang === ${JSONObject.quote(language)}") == "true"
        }
        evaluate("document.querySelectorAll('.settings > [role=tablist] button')[2].click(); null")
        waitUntil("unified data tab") { evaluate("Boolean(document.querySelector('.data-screen'))") == "true" }
        waitForIdle()
    }

    private fun assertControls(theme: String, language: String) {
        val state = JSONObject(decoded(evaluate("""
            JSON.stringify((()=>{
              const root=document.querySelector('.data-screen'),buttons=[...root.querySelectorAll('.data-actions button')];
              const boxes=[...buttons,...root.querySelectorAll('.data-mode')].map(e=>e.getBoundingClientRect());
              return {theme:document.documentElement.dataset.theme,language:document.documentElement.lang,
                viewport:innerWidth,width:root.getBoundingClientRect().width,scrollWidth:root.scrollWidth,
                actions:buttons.map(e=>e.textContent.trim()),modes:root.querySelectorAll('input[name=data-export-mode]').length,
                selected:root.querySelectorAll('input[name=data-export-mode]:checked').length,
                heights:boxes.map(e=>e.height),outside:boxes.some(e=>e.left< -1||e.right>innerWidth+1)};
            })())
        """.trimIndent())))
        assertEquals(theme, state.getString("theme"))
        assertEquals(language, state.getString("language"))
        assertTrue("Real mobile viewport: $state", state.getInt("viewport") in 240..600)
        assertTrue("Content must fit: $state", state.getDouble("width") <= state.getInt("viewport") + 1)
        assertTrue("Content must not overflow: $state", state.getInt("scrollWidth") <= state.getDouble("width") + 1)
        assertFalse("Controls must fit: $state", state.getBoolean("outside"))
        assertEquals(0, state.getInt("modes"))
        assertEquals(0, state.getInt("selected"))
        val labels = state.getJSONArray("actions")
        assertEquals("One export/share/import action group", 3, labels.length())
        assertEquals(if (language == "en") "Export file" else "Ekspor file", labels.getString(0))
        assertEquals(if (language == "en") "Import file" else "Impor file", labels.getString(2))
        val heights = state.getJSONArray("heights")
        for (index in 0 until heights.length()) assertTrue("44px touch targets: $state", heights.getDouble(index) >= 43.5)
        println("ANDROID_DATA_UI_LAYOUT_VERIFIED $state")
    }

    private fun assertPreviewLayout() {
        val state = JSONObject(decoded(evaluate("""
            JSON.stringify((()=>{const p=document.querySelector('[role=dialog]'),b=p.getBoundingClientRect();
              return {left:b.left,right:b.right,viewport:innerWidth,scrollWidth:p.scrollWidth,width:b.width,
                buttons:[...p.querySelectorAll('button')].map(e=>e.getBoundingClientRect().height),
                focusInside:p.contains(document.activeElement),counts:p.querySelectorAll('.stat-list__row').length};})())
        """.trimIndent())))
        assertTrue("Preview must fit: $state", state.getDouble("left") >= -1 && state.getDouble("right") <= state.getInt("viewport") + 1)
        assertTrue("Preview must not overflow: $state", state.getInt("scrollWidth") <= state.getDouble("width") + 1)
        assertTrue("Focus must enter preview", state.getBoolean("focusInside"))
        assertEquals("Created time and all four counts", 5, state.getInt("counts"))
        val heights = state.getJSONArray("buttons")
        for (index in 0 until heights.length()) assertTrue("Preview touch controls: $state", heights.getDouble(index) >= 43.5)
    }

    private fun clickImport() { evaluate("document.querySelectorAll('.data-actions button')[2].click(); null") }
    private fun waitForPreview(title: String) {
        waitUntil(title) { decoded(evaluate("document.querySelector('[role=dialog] h2')?.textContent || ''")) == title }
        waitForIdle()
    }
    private fun closePreview() {
        evaluate("document.querySelector('.sheet__footer button').click(); null")
        waitUntil("preview closed") { evaluate("!document.querySelector('[role=dialog]')") == "true" }
        waitForIdle()
    }
    private fun waitForIdle() {
        waitUntil("data controls available") { evaluate("document.querySelector('.data-actions button')?.disabled === false") == "true" ||
            evaluate("Boolean(document.querySelector('[role=dialog]')) && document.querySelector('.sheet__footer button')?.disabled === false") == "true" }
    }

    private fun selectedDocument(action: String, file: File, run: () -> Unit): Intent {
        assertEquals("app.vntracker.verificationandroid", target.packageName)
        val uri = FileProvider.getUriForFile(target, "${target.packageName}.fileprovider", file)
        assertEquals("content", uri.scheme)
        val launched = AtomicReference<Intent>()
        // Android SDK37 Instrumentation.ActivityMonitor.onStartActivity returns a
        // canned ActivityResult through the real ActivityResultRegistry callback.
        // React/IPC/storage are production; only system document selection is supplied.
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != action) return null
                launched.set(Intent(intent))
                return Instrumentation.ActivityResult(Activity.RESULT_OK, Intent().setData(uri))
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            run()
            assertNotNull("Production command must launch $action", launched.get())
            return launched.get()
        } finally {
            instrumentation.removeMonitor(monitor)
        }
    }

    private fun invoke(command: String, args: JSONObject = JSONObject()): Any? {
        val key = "__vnDataVerification"
        evaluate("window.$key=null;window.__TAURI_INTERNALS__.invoke(${JSONObject.quote(command)},$args).then(value=>window.$key={ok:true,value},error=>window.$key={ok:false,error});null")
        var response: JSONObject? = null
        waitUntil("IPC $command") {
            val raw = decoded(evaluate("JSON.stringify(window.$key)"))
            if (raw != "null") response = JSONObject(raw)
            response != null
        }
        assertTrue("IPC $command: $response", response!!.getBoolean("ok"))
        return response!!.opt("value").let { if (it == JSONObject.NULL) null else it }
    }
    private fun decoded(raw: String): String = (JSONTokener(raw).nextValue() as? String) ?: raw
    private fun evaluate(script: String): String {
        val latch = CountDownLatch(1)
        val result = AtomicReference<String>()
        instrumentation.runOnMainSync { webView.evaluateJavascript(script) { result.set(it); latch.countDown() } }
        assertTrue("WebView evaluation timed out", latch.await(15, TimeUnit.SECONDS))
        return result.get()
    }
    private fun waitUntil(label: String, condition: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (System.nanoTime() < deadline) {
            if (condition()) return
            Thread.sleep(100)
        }
        fail("Timed out waiting for $label")
    }
    private fun findWebView(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) for (index in 0 until view.childCount) findWebView(view.getChildAt(index))?.let { return it }
        return null
    }
}
