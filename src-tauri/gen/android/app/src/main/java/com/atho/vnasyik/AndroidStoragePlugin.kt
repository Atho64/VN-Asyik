package com.atho.vnasyik

import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileNotFoundException
import java.nio.ByteBuffer
import java.security.KeyStore
import java.util.concurrent.Executors
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class SecretArgs {
    lateinit var name: String
    var value: String? = null
}

@InvokeArg
class DocumentArgs {
    lateinit var filename: String
    lateinit var contents: String
}

@InvokeArg
class ReadUriArgs {
    lateinit var uri: String
}

/** Reject malformed UTF-8 before JSON parsing can replace stored titles or notes. */
internal fun decodeDocumentUtf8(bytes: ByteArray): String =
    Charsets.UTF_8.newDecoder().decode(ByteBuffer.wrap(bytes)).toString()

/** Ciphertext lives outside Android Auto Backup; the non-exportable key stays in Keystore. */
internal class SecureVault(private val directory: File, private val alias: String) {
    private fun file(name: String): AtomicFile {
        require(name in setOf("vndb", "chat", "tavily", "discord")) { "Unknown credential" }
        check(directory.isDirectory || directory.mkdirs()) { "Credential directory unavailable" }
        return AtomicFile(File(directory, "$name.enc"))
    }

    private fun key(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        check(create) { "Credential key unavailable; reconnect the account" }
        // API contract: Android SDK KeyGenParameterSpec, AES/GCM example (API 23+).
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build())
        }.generateKey()
    }

    @Synchronized
    fun write(name: String, value: String) {
        val target = file(name)
        require(value.isNotBlank() && value.length <= 4096) { "Invalid credential" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key(true))
        cipher.updateAAD(name.toByteArray(Charsets.UTF_8))
        val encrypted = cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        val stream = target.startWrite()
        try {
            stream.write(cipher.iv.size)
            stream.write(cipher.iv)
            stream.write(encrypted)
            target.finishWrite(stream)
        } catch (error: Exception) {
            target.failWrite(stream)
            throw error
        }
    }

    @Synchronized
    fun read(name: String): String? {
        val target = file(name)
        val bytes = try { target.readFully() } catch (_: FileNotFoundException) { return null }
        check(bytes.size in 30..8192 && bytes[0].toInt() == 12) { "Invalid credential data" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(false), GCMParameterSpec(128, bytes.copyOfRange(1, 13)))
        cipher.updateAAD(name.toByteArray(Charsets.UTF_8))
        return String(cipher.doFinal(bytes.copyOfRange(13, bytes.size)), Charsets.UTF_8)
    }

    @Synchronized
    fun remove(name: String) {
        val target = file(name)
        target.delete()
        check(!target.baseFile.exists()) { "Could not remove credential" }
    }
}

@TauriPlugin
class AndroidStoragePlugin(private val activity: Activity) : Plugin(activity) {
    private val vault by lazy {
        SecureVault(File(activity.noBackupFilesDir, "credentials"), "vn-asyik.credentials.v1")
    }
    private val io = Executors.newSingleThreadExecutor { task ->
        Thread(task, "vn-asyik-storage").apply { isDaemon = true }
    }

    @Command
    fun readSecret(invoke: Invoke) = io.execute {
        try {
            val value = vault.read(invoke.parseArgs(SecretArgs::class.java).name)
            invoke.resolve(JSObject().put("value", value ?: org.json.JSONObject.NULL))
        } catch (_: Exception) {
            invoke.reject("Secure storage could not read the credential; reconnect the account")
        }
    }

    /**
     * Copy a `content://` document the system picker returned into a private
     * cache file and answer with its path and display name. SAF documents have
     * no filesystem path; the content resolver is the only safe reader (the
     * Rust side never treats the URI as one). The display name is queried from
     * OpenableColumns because the URI itself is an opaque id: multi-file
     * imports are ordered by the name people see, never by the id. The caller
     * owns the file and deletes it when done, so the cache never grows with
     * every pick.
     */
    @Command
    fun readUriToCache(invoke: Invoke) = io.execute {
        try {
            val uri = invoke.parseArgs(ReadUriArgs::class.java).uri
            require(uri.startsWith("content://")) { "Not a content URI" }
            val parsed = android.net.Uri.parse(uri)
            val picked = File(activity.cacheDir, "picked").apply { mkdirs() }
            val target = File.createTempFile("pick", ".bin", picked)
            try {
                activity.contentResolver.openInputStream(parsed).use { input ->
                    requireNotNull(input) { "The document is no longer available" }
                    target.outputStream().use { output ->
                        val buffer = ByteArray(8192)
                        var total = 0L
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            total += count
                            check(total <= 64L * 1024 * 1024) { "The file is larger than 64 MiB" }
                            output.write(buffer, 0, count)
                        }
                    }
                }
            } catch (error: Exception) {
                target.delete()
                throw error
            }
            invoke.resolve(
                JSObject()
                    .put("path", target.absolutePath)
                    .put("name", queryDisplayName(parsed))
            )
        } catch (error: Exception) {
            // Every failure above carries a specific message (not a content
            // URI, document gone, over 64 MiB); surface it instead of one
            // generic sentence that blames the size for every failure.
            invoke.reject(error.message ?: "The selected document could not be read")
        }
    }

    /** The document's human-facing name, or null when the provider will not say. */
    private fun queryDisplayName(uri: android.net.Uri): String? = try {
        activity.contentResolver
            .query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
    } catch (_: Exception) {
        null
    }

    @Command
    fun writeSecret(invoke: Invoke) = io.execute {
        try {
            val args = invoke.parseArgs(SecretArgs::class.java)
            vault.write(args.name, requireNotNull(args.value))
            invoke.resolve()
        } catch (_: Exception) {
            invoke.reject("Secure storage could not save the credential")
        }
    }

    @Command
    fun removeSecret(invoke: Invoke) = io.execute {
        try {
            vault.remove(invoke.parseArgs(SecretArgs::class.java).name)
            invoke.resolve()
        } catch (_: Exception) {
            invoke.reject("Secure storage could not remove the credential")
        }
    }

    @Command
    fun saveDocument(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(DocumentArgs::class.java)
            val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "application/json"
                putExtra(Intent.EXTRA_TITLE, args.filename)
            }
            startActivityForResult(invoke, intent, "saveDocumentResult")
        } catch (_: Exception) {
            invoke.reject("The document picker could not be opened")
        }
    }

    @ActivityCallback
    fun saveDocumentResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.resolve(JSObject().put("saved", false))
            return
        }
        io.execute {
            try {
                val uri = requireNotNull(result.data?.data)
                val args = invoke.parseArgs(DocumentArgs::class.java)
                requireNotNull(activity.contentResolver.openOutputStream(uri, "wt")).use {
                    it.write(args.contents.toByteArray(Charsets.UTF_8))
                }
                invoke.resolve(JSObject().put("saved", true))
            } catch (_: Exception) {
                invoke.reject("The selected document could not be saved")
            }
        }
    }

    @Command
    fun openDocument(invoke: Invoke) {
        try {
            val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                type = "*/*"
            }
            startActivityForResult(invoke, intent, "openDocumentResult")
        } catch (_: Exception) {
            invoke.reject("The document picker could not be opened")
        }
    }

    @ActivityCallback
    fun openDocumentResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.resolve(JSObject().put("contents", org.json.JSONObject.NULL))
            return
        }
        io.execute {
            try {
                val uri = requireNotNull(result.data?.data)
                val output = ByteArrayOutputStream()
                requireNotNull(activity.contentResolver.openInputStream(uri)).use { input ->
                    val buffer = ByteArray(8192)
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        check(output.size() + count <= 64 * 1024 * 1024) { "File exceeds 64 MiB" }
                        output.write(buffer, 0, count)
                    }
                }
                invoke.resolve(JSObject().put("contents", decodeDocumentUtf8(output.toByteArray())))
            } catch (_: Exception) {
                invoke.reject("The selected document could not be read (maximum 64 MiB)")
            }
        }
    }

    @Command
    fun shareDocument(invoke: Invoke) = io.execute {
        try {
            val args = invoke.parseArgs(DocumentArgs::class.java)
            require(args.filename.matches(Regex("[a-zA-Z0-9_.-]+\\.json")))
            val directory = File(activity.cacheDir, "transfers")
            check(directory.isDirectory || directory.mkdirs())
            val file = File(directory, args.filename)
            file.writeText(args.contents, Charsets.UTF_8)
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
            val send = Intent(Intent.ACTION_SEND).apply {
                type = "application/json"
                putExtra(Intent.EXTRA_STREAM, uri)
                clipData = ClipData.newRawUri("VN Asyik", uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            activity.runOnUiThread {
                try {
                    activity.startActivity(Intent.createChooser(send, null))
                    invoke.resolve()
                } catch (_: Exception) {
                    invoke.reject("The system share sheet could not be opened")
                }
            }
        } catch (_: Exception) {
            invoke.reject("The system share sheet could not be opened")
        }
    }
}
