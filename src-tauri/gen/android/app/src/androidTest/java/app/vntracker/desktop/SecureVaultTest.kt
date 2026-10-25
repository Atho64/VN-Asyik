package app.vntracker.desktop

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.security.KeyStore
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class SecureVaultTest {
    @Test
    fun credentialsAreEncryptedPersistedAuthenticatedAndRemovable() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val id = UUID.randomUUID().toString()
        val directory = File(context.cacheDir, "vault-test-$id")
        val alias = "vn-asyik.test.$id"
        try {
            val secret = "synthetic-credential-for-device-test"
            val vault = SecureVault(directory, alias)
            assertNull(vault.read("vndb"))
            vault.write("vndb", secret)
            val encrypted = File(directory, "vndb.enc").readBytes()
            assertFalse(String(encrypted, Charsets.ISO_8859_1).contains(secret))
            assertEquals(secret, SecureVault(directory, alias).read("vndb"))
            // AtomicFile must recover a retained backup after an interrupted write.
            val base = File(directory, "vndb.enc")
            assertTrue(base.renameTo(File(directory, "vndb.enc.bak")))
            assertEquals(secret, vault.read("vndb"))
            // Swapping ciphertext between accounts must fail, even under the same key.
            File(directory, "chat.enc").writeBytes(encrypted)
            try {
                vault.read("chat")
                fail("Account-bound GCM authentication must reject swapped ciphertext")
            } catch (_: java.security.GeneralSecurityException) { }
            vault.remove("vndb")
            assertNull(vault.read("vndb"))
        } finally {
            directory.listFiles()?.forEach { it.delete() }
            directory.delete()
            KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) }
        }
    }
}
