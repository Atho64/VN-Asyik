package app.vntracker.desktop

/** Removes unrelated app identities before any usage event crosses into Rust. */
internal object UsageEventPolicy {
    const val SELF_PACKAGE_MARKER = "vn-asyik:self"

    fun payload(
        wanted: Set<String>,
        selfPackage: String,
        packageName: String?,
        activityName: String?,
        kind: Int,
        atMs: Long,
    ): Map<String, Any?>? {
        val global = kind in setOf(15, 16, 17, 18, 26, 27)
        val requested = packageName in wanted
        if (!global && kind !in setOf(1, 2, 23)) return null
        if (!global && !requested && kind != 1) return null
        return mapOf(
            "package" to when {
                global -> null
                requested -> packageName
                packageName == selfPackage -> SELF_PACKAGE_MARKER
                else -> "" // An anonymous other-app resume closes the current interval.
            },
            "activity" to if (requested && !global) activityName else null,
            "kind" to kind,
            "atMs" to atMs,
        )
    }
}
