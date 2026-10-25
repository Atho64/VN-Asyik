package com.atho.vnasyik

import android.app.Activity
import android.os.Handler
import android.os.Looper
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

@InvokeArg
internal class PresenceUpdateArgs {
    var token: String? = null
    var clientId: String? = null
    var name: String? = null
    var details: String? = null
    var state: String? = null
    var startTimestamp: Long? = null
    var largeImage: String? = null
    var largeText: String? = null
    var smallImage: String? = null
    var smallText: String? = null
    var buttons: Array<String>? = null
    var buttonUrls: Array<String>? = null
}

@TauriPlugin
class DiscordGatewayPlugin(private val activity: Activity) : Plugin(activity) {

    companion object {
        private const val GATEWAY_URL = "wss://gateway.discord.gg/?v=10&encoding=json"
        private const val REST_BASE = "https://discord.com/api/v10"
        private const val OP_DISPATCH = 0
        private const val OP_HEARTBEAT = 1
        private const val OP_IDENTIFY = 2
        private const val OP_PRESENCE_UPDATE = 3
        private const val OP_RESUME = 6
        private const val OP_RECONNECT = 7
        private const val OP_INVALID_SESSION = 9
        private const val OP_HELLO = 10
        private const val OP_HEARTBEAT_ACK = 11

        /** A failed external-asset lookup is retried only after this long, so a
         * flaky endpoint does not turn every presence poll into an HTTP attempt. */
        private const val ASSET_RETRY_MS = 5 * 60 * 1000L
    }

    private val mainHandler = Handler(Looper.getMainLooper())
    private val reconnectRunnable = Runnable { connectGateway() }
    private var okHttpClient: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build()

    /** Bounded timeouts: unlike the gateway socket, the REST proxy call must
     * fail fast so a presence update is never stuck behind it. */
    private var restClient: OkHttpClient = okHttpClient.newBuilder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    /** Image URL → `mp:external/...` key, so the poll loop hits a map, not the API. */
    private val assetKeyCache = java.util.concurrent.ConcurrentHashMap<String, String>()
    private val assetFailureAt = java.util.concurrent.ConcurrentHashMap<String, Long>()

    private var webSocket: WebSocket? = null
    private var heartbeatExecutor: ScheduledExecutorService? = null
    private var heartbeatFuture: ScheduledFuture<*>? = null

    private var token: String? = null
    private var clientId: String? = null
    private var lastActivityJson: JSONObject? = null

    private var sequence: Int? = null
    private var sessionId: String? = null
    private var resumeGatewayUrl: String? = null

    private var isConnected = false
    private var isConnecting = false
    private var shouldStayConnected = false
    private var lastError: String? = null

    @Synchronized
    private fun connectGateway() {
        mainHandler.removeCallbacks(reconnectRunnable)
        if (isConnected || isConnecting) return
        val currentToken = token
        if (currentToken.isNullOrBlank()) {
            lastError = "Missing Discord user token"
            return
        }

        isConnecting = true
        lastError = null

        val targetUrl = resumeGatewayUrl ?: GATEWAY_URL
        val request = Request.Builder().url(targetUrl).build()

        webSocket = okHttpClient.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                // Connection opened; wait for Opcode 10 (HELLO) to send IDENTIFY/RESUME
            }

            override fun onMessage(ws: WebSocket, text: String) {
                try {
                    handleGatewayMessage(ws, JSONObject(text))
                } catch (e: Exception) {
                    synchronized(this@DiscordGatewayPlugin) {
                        if (ws === webSocket) {
                            lastError = "Gateway parse error: ${e.message}"
                        }
                    }
                }
            }

            override fun onClosing(ws: WebSocket, code: Int, reason: String) {
                ws.close(code, reason)
            }

            override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                synchronized(this@DiscordGatewayPlugin) {
                    if (ws !== webSocket) return
                    isConnected = false
                    isConnecting = false
                    stopHeartbeat()
                    webSocket = null

                    if (code == 4004) {
                        lastError = "Authentication failed (invalid Discord token)"
                        shouldStayConnected = false
                        sessionId = null
                        sequence = null
                        resumeGatewayUrl = null
                        PresenceForegroundService.stop(activity)
                    } else if (shouldStayConnected) {
                        mainHandler.removeCallbacks(reconnectRunnable)
                        mainHandler.postDelayed(reconnectRunnable, 5000)
                    }
                }
            }

            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                synchronized(this@DiscordGatewayPlugin) {
                    if (ws !== webSocket) return
                    isConnected = false
                    isConnecting = false
                    stopHeartbeat()
                    webSocket = null
                    lastError = t.message ?: "Connection failure"

                    if (shouldStayConnected) {
                        mainHandler.removeCallbacks(reconnectRunnable)
                        mainHandler.postDelayed(reconnectRunnable, 5000)
                    }
                }
            }
        })
    }

    @Synchronized
    private fun handleGatewayMessage(ws: WebSocket, json: JSONObject) {
        if (ws !== webSocket) return
        val op = json.optInt("op", -1)
        if (json.has("s") && !json.isNull("s")) {
            sequence = json.getInt("s")
        }

        when (op) {
            OP_HELLO -> {
                val d = json.getJSONObject("d")
                val interval = d.getLong("heartbeat_interval")
                startHeartbeat(ws, interval)

                if (sequence != null && !sessionId.isNullOrBlank()) {
                    sendResume(ws)
                } else {
                    sendIdentify(ws)
                }
            }
            OP_DISPATCH -> {
                val t = json.optString("t", "")
                if (t == "READY") {
                    val d = json.getJSONObject("d")
                    sessionId = d.optString("session_id")
                    val rawResumeUrl = d.optString("resume_gateway_url")
                    resumeGatewayUrl = if (!rawResumeUrl.isNullOrBlank()) {
                        "${rawResumeUrl.trimEnd('/')}/?v=10&encoding=json"
                    } else {
                        null
                    }
                    isConnected = true
                    isConnecting = false
                    lastError = null
                    lastActivityJson?.let { sendPresence(ws, it) }
                } else if (t == "RESUMED") {
                    isConnected = true
                    isConnecting = false
                    lastError = null
                    lastActivityJson?.let { sendPresence(ws, it) }
                }
            }
            OP_HEARTBEAT -> {
                sendHeartbeat(ws)
            }
            OP_RECONNECT -> {
                ws.close(4000, "Discord requested reconnect")
            }
            OP_INVALID_SESSION -> {
                val resumable = json.optBoolean("d", false)
                if (!resumable) {
                    sessionId = null
                    sequence = null
                    resumeGatewayUrl = null
                }
                mainHandler.postDelayed({
                    synchronized(this@DiscordGatewayPlugin) {
                        if (ws !== webSocket) return@synchronized
                        if (resumable && sessionId != null) sendResume(ws) else sendIdentify(ws)
                    }
                }, 500)
            }
            OP_HEARTBEAT_ACK -> {
                // Heartbeat acknowledged
            }
        }
    }

    private fun startHeartbeat(ws: WebSocket, intervalMs: Long) {
        stopHeartbeat()
        heartbeatExecutor = Executors.newSingleThreadScheduledExecutor()
        heartbeatFuture = heartbeatExecutor?.scheduleAtFixedRate({
            synchronized(this@DiscordGatewayPlugin) {
                if (ws === webSocket) {
                    sendHeartbeat(ws)
                }
            }
        }, intervalMs, intervalMs, TimeUnit.MILLISECONDS)
    }

    private fun stopHeartbeat() {
        heartbeatFuture?.cancel(true)
        heartbeatFuture = null
        heartbeatExecutor?.shutdownNow()
        heartbeatExecutor = null
    }

    @Synchronized
    private fun sendHeartbeat(ws: WebSocket) {
        val hb = JSONObject().apply {
            put("op", OP_HEARTBEAT)
            put("d", sequence ?: JSONObject.NULL)
        }
        ws.send(hb.toString())
    }

    @Synchronized
    private fun sendIdentify(ws: WebSocket) {
        val currentToken = token ?: return
        val id = JSONObject().apply {
            put("op", OP_IDENTIFY)
            put("d", JSONObject().apply {
                put("token", currentToken)
                put("capabilities", 65)
                put("compress", false)
                put("properties", JSONObject().apply {
                    put("os", "Android")
                    put("browser", "Discord Android")
                    put("device", "VN Asyik")
                })
            })
        }
        ws.send(id.toString())
    }

    @Synchronized
    private fun sendResume(ws: WebSocket) {
        val currentToken = token ?: return
        val sId = sessionId ?: return
        val res = JSONObject().apply {
            put("op", OP_RESUME)
            put("d", JSONObject().apply {
                put("token", currentToken)
                put("session_id", sId)
                put("seq", sequence ?: 0)
            })
        }
        ws.send(res.toString())
    }

    /**
     * Converts a public image URL into Discord's `mp:external/...` media-proxy key.
     *
     * Gateway presence renders only application asset keys and media-proxy keys:
     * a raw URL in `large_image` is silently ignored, which is why the card had
     * no artwork on Android. `POST /applications/{id}/external-assets` is the
     * proxy call the Discord client itself makes; it is resolved once per URL
     * here and cached for the session. Any failure falls back to the raw URL,
     * which is exactly the previous behaviour.
     */
    private fun resolveExternalAsset(url: String, authToken: String?, clientId: String?): String {
        val isHttp = url.startsWith("http://") || url.startsWith("https://")
        if (!isHttp || url.length > 256) return url
        if (clientId.isNullOrBlank() || authToken.isNullOrBlank()) return url
        assetKeyCache[url]?.let { return it }
        val failedAt = assetFailureAt[url]
        if (failedAt != null && System.currentTimeMillis() - failedAt < ASSET_RETRY_MS) return url
        return try {
            val payload = JSONObject().put("urls", JSONArray().put(url)).toString()
            val request = Request.Builder()
                .url("$REST_BASE/applications/$clientId/external-assets")
                .header("Authorization", authToken)
                .post(payload.toRequestBody("application/json".toMediaType()))
                .build()
            restClient.newCall(request).execute().use { response ->
                val text = response.body?.string().orEmpty()
                val path = if (response.isSuccessful) {
                    runCatching {
                        val arr = JSONArray(text)
                        if (arr.length() > 0) arr.getJSONObject(0).optString("external_asset_path", "") else ""
                    }.getOrDefault("")
                } else ""
                if (path.isBlank()) {
                    assetFailureAt[url] = System.currentTimeMillis()
                    url
                } else {
                    val key = "mp:$path"
                    assetKeyCache[url] = key
                    assetFailureAt.remove(url)
                    key
                }
            }
        } catch (e: Exception) {
            assetFailureAt[url] = System.currentTimeMillis()
            url
        }
    }

    private fun sendPresence(ws: WebSocket, activity: JSONObject?) {
        var startMs = -1L
        val d = JSONObject().apply {
            val activitiesArr = JSONArray()
            if (activity != null) {
                startMs = activity.optLong("startTimestamp", -1L)
                val act = JSONObject().apply {
                    put("name", activity.optString("name", "VN Asyik"))
                    put("type", 0)
                    if (activity.has("details") && !activity.isNull("details")) {
                        val det = activity.getString("details")
                        if (det.isNotBlank()) put("details", det)
                    }
                    if (activity.has("state") && !activity.isNull("state")) {
                        val st = activity.getString("state")
                        if (st.isNotBlank()) put("state", st)
                    }

                    if (startMs > 0) {
                        put("timestamps", JSONObject().put("start", startMs))
                    }

                    val assets = JSONObject()
                    var hasAssets = false
                    val cid = activity.optString("clientId", "")
                    val authToken = token
                    // A raw URL is invisible on the gateway; every http(s) image
                    // goes through the media proxy first (cached per URL).
                    if (activity.has("largeImage") && !activity.isNull("largeImage")) {
                        val img = activity.getString("largeImage")
                        if (img.isNotBlank()) {
                            assets.put("large_image", resolveExternalAsset(img, authToken, cid))
                            hasAssets = true
                        }
                    }
                    if (activity.has("largeText") && !activity.isNull("largeText")) {
                        val txt = activity.getString("largeText")
                        if (txt.isNotBlank()) {
                            assets.put("large_text", txt)
                            hasAssets = true
                        }
                    }
                    if (activity.has("smallImage") && !activity.isNull("smallImage")) {
                        val img = activity.getString("smallImage")
                        if (img.isNotBlank()) {
                            assets.put("small_image", resolveExternalAsset(img, authToken, cid))
                            hasAssets = true
                        }
                    }
                    if (activity.has("smallText") && !activity.isNull("smallText")) {
                        val txt = activity.getString("smallText")
                        if (txt.isNotBlank()) {
                            assets.put("small_text", txt)
                            hasAssets = true
                        }
                    }
                    if (hasAssets) {
                        put("assets", assets)
                    }

                    val buttons = activity.optJSONArray("buttons")
                    val buttonUrls = activity.optJSONArray("buttonUrls")
                    if (buttons != null && buttonUrls != null && buttons.length() > 0 && buttons.length() == buttonUrls.length()) {
                        val btnArr = JSONArray()
                        val urlArr = JSONArray()
                        val limit = minOf(buttons.length(), 2)
                        for (i in 0 until limit) {
                            btnArr.put(buttons.getString(i))
                            urlArr.put(buttonUrls.getString(i))
                        }
                        put("buttons", btnArr)
                        put("metadata", JSONObject().put("button_urls", urlArr))
                    }

                    if (cid.isNotBlank()) {
                        put("application_id", cid)
                    }
                }
                activitiesArr.put(act)
            }
            put("activities", activitiesArr)
            put("status", "online")
            put("afk", false)
            put("since", if (startMs > 0) startMs else System.currentTimeMillis())
        }

        val update = JSONObject().apply {
            put("op", OP_PRESENCE_UPDATE)
            put("d", d)
        }
        ws.send(update.toString())
    }

    @Synchronized
    private fun disconnectInternal() {
        shouldStayConnected = false
        mainHandler.removeCallbacks(reconnectRunnable)
        stopHeartbeat()
        webSocket?.close(1000, "Normal closure")
        webSocket = null
        isConnected = false
        isConnecting = false
        lastActivityJson = null
        PresenceForegroundService.stop(activity)
    }

    @Command
    fun update(invoke: Invoke) {
        val args = invoke.parseArgs(PresenceUpdateArgs::class.java)
        val t = args.token?.trim()
        if (t.isNullOrBlank()) {
            invoke.reject("Missing Discord token")
            return
        }

        synchronized(this) {
            if (token != null && token != t) {
                disconnectInternal()
                sessionId = null
                sequence = null
                resumeGatewayUrl = null
            }
            token = t
            clientId = args.clientId?.trim()
            shouldStayConnected = true

            val actObj = JSONObject().apply {
                put("name", args.name?.takeIf { it.isNotBlank() } ?: "VN Asyik")
                args.details?.let { put("details", it) }
                args.state?.let { put("state", it) }
                args.startTimestamp?.let { put("startTimestamp", it) }
                args.largeImage?.let { put("largeImage", it) }
                args.largeText?.let { put("largeText", it) }
                args.smallImage?.let { put("smallImage", it) }
                args.smallText?.let { put("smallText", it) }
                args.buttons?.let { put("buttons", JSONArray(it.toList())) }
                args.buttonUrls?.let { put("buttonUrls", JSONArray(it.toList())) }
                clientId?.let { put("clientId", it) }
            }

            lastActivityJson = actObj

            val title = actObj.optString("name", "VN Asyik")
            val details = actObj.optString("details", "")
            val notifText = if (details.isNotBlank() && details != title) {
                "Playing $title • $details"
            } else {
                "Playing $title"
            }
            PresenceForegroundService.start(activity, title, notifText)

            val ws = webSocket
            if (isConnected && ws != null) {
                sendPresence(ws, actObj)
            } else {
                connectGateway()
            }
        }
        invoke.resolve()
    }

    @Command
    fun clear(invoke: Invoke) {
        synchronized(this) {
            val ws = webSocket
            if (isConnected && ws != null) {
                sendPresence(ws, null)
            }
            disconnectInternal()
        }
        invoke.resolve()
    }

    @Command
    fun stop(invoke: Invoke) {
        synchronized(this) {
            disconnectInternal()
        }
        invoke.resolve()
    }

    @Command
    fun status(invoke: Invoke) {
        val result = JSObject()
        synchronized(this) {
            when {
                isConnected -> {
                    result.put("status", "connected")
                }
                isConnecting -> {
                    result.put("status", "connecting")
                }
                lastError != null -> {
                    result.put("status", "error")
                    result.put("message", lastError)
                }
                else -> {
                    result.put("status", "disconnected")
                }
            }
        }
        invoke.resolve(result)
    }
}
