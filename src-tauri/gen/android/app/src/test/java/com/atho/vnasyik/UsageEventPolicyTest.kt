package com.atho.vnasyik

import org.junit.Assert.*
import org.junit.Test

class UsageEventPolicyTest {
    private val wanted = setOf("org.example.runner")
    private val selfPackage = "com.atho.vnasyik"

    @Test
    fun unrelatedResumeClosesTimeWithoutExposingAppIdentity() {
        val result = UsageEventPolicy.payload(wanted, selfPackage, "com.private.browser", "SecretActivity", 1, 100L)!!
        assertEquals("", result["package"])
        assertNull(result["activity"])
        assertEquals(1, result["kind"])
        assertNull(UsageEventPolicy.payload(wanted, selfPackage, "com.private.browser", "SecretActivity", 2, 100L))
    }

    @Test
    fun selectedActivityAndAnonymousSelfReturnAreRetained() {
        val runner = UsageEventPolicy.payload(wanted, selfPackage, "org.example.runner", "GameActivity", 1, 100L)!!
        assertEquals("org.example.runner", runner["package"])
        assertEquals("GameActivity", runner["activity"])
        val self = UsageEventPolicy.payload(wanted, selfPackage, selfPackage, "MainActivity", 1, 101L)!!
        assertEquals(UsageEventPolicy.SELF_PACKAGE_MARKER, self["package"])
        assertNull(self["activity"])
    }

    @Test
    fun deviceAndKeyguardEventsCarryNoIdentityOrOtherEventTypes() {
        for (kind in listOf(15, 16, 17, 18, 26, 27)) {
            val event = UsageEventPolicy.payload(wanted, selfPackage, "android", "Internal", kind, 100L)!!
            assertNull(event["package"])
            assertNull(event["activity"])
        }
        assertNull(UsageEventPolicy.payload(wanted, selfPackage, "org.example.runner", "GameActivity", 7, 100L))
    }
}
