package com.atho.vnasyik

import android.app.Activity
import android.content.Intent
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.tauri.plugin.Invoke
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.RandomAccessFile
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Exercises content-resolver callbacks only; no document is shared externally. */
@RunWith(AndroidJUnit4::class)
class AndroidDocumentTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun contentUriRoundtripRejectsMalformedAndOversizedDocuments() {
        val context = instrumentation.targetContext
        assertEquals("This regression must use the isolated verification APK",
            "app.vntracker.verificationandroid", context.packageName)
        val activity = instrumentation.startActivitySync(
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        val plugin = AndroidStoragePlugin(activity)
        val directory = File(context.cacheDir, "document-test-${UUID.randomUUID()}")
        assertTrue(directory.mkdirs())
        try {
            val file = File(directory, "synthetic.json")
            file.writeText("obsolete-content-that-must-be-truncated".repeat(100))
            // FileProvider.getUriForFile and ContentResolver's wt/r modes are
            // verified against AndroidX source and Android SDK 37 source.
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
            assertEquals("content", uri.scheme)
            val selected = ActivityResult(Activity.RESULT_OK, Intent().setData(uri))
            val contents = """{"title":"日本語・Indonesia 😀","notes":"synthetic only"}"""
            val args = JSONObject().put("filename", file.name).put("contents", contents)
            val saved = call("saveDocument", args) { plugin.saveDocumentResult(it, selected) }
            assertTrue(saved.first)
            assertTrue(saved.second.getBoolean("saved"))
            val opened = call("openDocument") { plugin.openDocumentResult(it, selected) }
            assertTrue(opened.first)
            assertEquals(contents, opened.second.getString("contents"))
            assertEquals(contents.toByteArray(Charsets.UTF_8).size.toLong(), file.length())

            file.writeBytes(byteArrayOf(0xc3.toByte(), 0x28))
            val malformed = call("openDocument") { plugin.openDocumentResult(it, selected) }
            assertFalse("Malformed UTF-8 must never silently replace stored text", malformed.first)

            // A sparse file crosses the production 64 MiB boundary without
            // allocating a huge test string or leaving data in shared storage.
            val oversizedBytes = 64L * 1024 * 1024 + 1
            // Reset the invalid bytes first: NUL padding is valid UTF-8, so
            // rejection here must come from size rather than the prior case.
            file.writeBytes(byteArrayOf(0x61))
            RandomAccessFile(file, "rw").use { it.setLength(oversizedBytes) }
            val oversized = call("openDocument") { plugin.openDocumentResult(it, selected) }
            assertFalse("Files exceeding 64 MiB must be rejected", oversized.first)
            assertTrue(oversized.second.getString("message").contains("64 MiB"))

            val cancelled = ActivityResult(Activity.RESULT_CANCELED, null)
            val cancelSave = call("saveDocument", args) { plugin.saveDocumentResult(it, cancelled) }
            assertTrue(cancelSave.first)
            assertFalse(cancelSave.second.getBoolean("saved"))
            val cancelOpen = call("openDocument") { plugin.openDocumentResult(it, cancelled) }
            assertTrue(cancelOpen.first)
            assertTrue(cancelOpen.second.isNull("contents"))
            println("ANDROID_DOCUMENT_VERIFIED contentUri=true unicodeRoundtrip=true truncate=true malformedRejected=true oversizedBytes=$oversizedBytes cancelled=true")
        } finally {
            directory.listFiles()?.forEach { it.delete() }
            directory.delete()
        }
    }

    private fun call(command: String, args: JSONObject = JSONObject(),
        execute: (Invoke) -> Unit): Pair<Boolean, JSONObject> {
        val latch = CountDownLatch(1)
        val result = AtomicReference<Pair<Boolean, JSONObject>>()
        val response: (Long, String) -> Unit = { callback, data ->
            result.set(Pair(callback == 1L, JSONObject(data)))
            latch.countDown()
        }
        // Tauri 2.12.4 Invoke constructor verified from its shipped source.
        val mapper = Class.forName("com.fasterxml.jackson.databind.ObjectMapper")
            .getDeclaredConstructor().newInstance()
        val invoke = Invoke::class.java.constructors.single().newInstance(
            1L, command, 1L, 2L, response, args.toString(), mapper,
        ) as Invoke
        execute(invoke)
        assertTrue("Native document callback timed out", latch.await(30, TimeUnit.SECONDS))
        return result.get()
    }
}
