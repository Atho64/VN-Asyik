package app.vntracker.desktop

import android.app.Activity
import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Real WebView regression; only the isolated APK with 1,500 synthetic cached games. */
@RunWith(AndroidJUnit4::class)
class AndroidLibraryUiTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val target = instrumentation.targetContext
    private lateinit var activity: Activity
    private lateinit var webView: WebView

    @Test
    fun largeLibraryLayoutsAndExplicitPolicyInDarkAndLight() {
        assertEquals("Never run UI fixtures against user data", "app.vntracker.verificationandroid", target.packageName)
        activity = instrumentation.startActivitySync(
            Intent(target, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        waitUntil("Tauri WebView") {
            instrumentation.runOnMainSync {
                findWebView(activity.window.decorView)?.let { webView = it }
            }
            ::webView.isInitialized && evaluate("Boolean(window.__TAURI_INTERNALS__?.invoke)") == "true"
        }
        waitUntil("1,500 synthetic library games") {
            evaluate("document.querySelector('.library__filters .library__count')?.textContent?.includes('1500') || false") == "true"
        }
        val previous = invoke("get_prefs")
        try {
            for (theme in listOf("dark", "light")) {
                invoke("set_theme", JSONObject().put("value", theme))
                reloadLibrary(theme)
                for (view in listOf("grid", "list", "shelf")) {
                    selectView(view)
                    assertLayout(theme, view)
                    assertExplicitConcealedAndRevealed(view)
                    scrollFar(view)
                    waitUntil("later virtual items in $view") {
                        json("[...document.querySelectorAll('[data-library-game]')].map(e=>Number(e.dataset.cardIndex))")
                            .let { values -> values.length() > 0 && values.getInt(0) > 100 }
                    }
                    assertLayout(theme, view)
                    // Synthetic keyboard events test React navigation through the
                    // real Android WebView even on a device with no physical keyboard.
                    evaluate("(()=>{const a=document.querySelector('[data-library-game] a');a.focus({preventScroll:true});a.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));return null})()")
                    waitUntil("keyboard End reaches game 1,500") {
                        evaluate("document.activeElement?.closest('[data-library-game]')?.dataset.cardIndex === '1499'") == "true"
                    }
                    assertLayout(theme, view)
                }
            }
        } finally {
            // Restore only test preferences; no library row or credential changes.
            invoke("set_theme", JSONObject().put("value", previous.getString("theme")))
            invoke("set_nsfw_mode", JSONObject().put("value", previous.getString("nsfw")))
            reloadLibrary(previous.getString("theme"))
            selectView("list")
        }
    }

    private fun selectView(view: String) {
        val index = listOf("grid", "list", "shelf").indexOf(view)
        evaluate("document.querySelectorAll('.library__bar [role=tab]')[$index].click(); null")
        waitUntil("$view layout") {
            evaluate("document.querySelector('.library-virtual')?.dataset.view === ${JSONObject.quote(view)} && document.querySelectorAll('[data-library-game]').length > 0") == "true"
        }
        settle()
    }

    private fun assertLayout(theme: String, view: String) {
        val gapToken = if (view == "list") "--space-2" else "--space-4"
        val state = JSONObject(decoded(evaluate("""
          JSON.stringify((()=>{
            const host=document.querySelector('.library-virtual');
            const main=document.querySelector('.app__main');
            const items=[...host.querySelectorAll('[data-library-game]')];
            const rows=[...host.querySelectorAll('.library-virtual__row')].sort((a,b)=>Number(a.dataset.index)-Number(b.dataset.index));
            const gap=parseFloat(getComputedStyle(document.documentElement).getPropertyValue('$gapToken'));
            const errors=[];
            if(!Number.isFinite(gap))errors.push({invalidGap:true});
            for(let i=1;i<rows.length;i++){
              if(Number(rows[i].dataset.index)!==Number(rows[i-1].dataset.index)+1)continue;
              const a=rows[i-1].getBoundingClientRect(),b=rows[i].getBoundingClientRect();
              const distance=host.dataset.view==='shelf'?b.left-a.right:b.top-a.bottom;
              if(!Number.isFinite(distance) || distance < -1.1 || Math.abs(distance-gap)>2.1)errors.push({index:rows[i].dataset.index,distance,gap});
            }
            const last=rows.at(-1),track=host.querySelector('.library-virtual__track');
            const lastBox=last?.getBoundingClientRect(),trackBox=track?.getBoundingClientRect();
            return {theme:document.documentElement.dataset.theme,view:host.dataset.view,count:items.length,
              viewport:innerWidth,mainWidth:main.clientWidth,hostWidth:host.clientWidth,errors,
              trackContainsRows:host.dataset.view==='shelf'?trackBox.bottom+1>=lastBox.bottom:trackBox.bottom+1>=lastBox.bottom};
          })())
        """.trimIndent())))
        assertEquals(theme, state.getString("theme"))
        assertEquals(view, state.getString("view"))
        assertTrue("Real mobile viewport: $state", state.getInt("viewport") in 240..600)
        assertTrue("Bounded DOM: $state", state.getInt("count") in 1..99)
        assertTrue("Main content width must fit the viewport", state.getInt("mainWidth") <= state.getInt("viewport"))
        assertTrue("Virtual rows must be contained in their extent: $state", state.getBoolean("trackContainsRows"))
        assertEquals("Measured rows must retain token gaps without overlaps: $state", 0, state.getJSONArray("errors").length())
        println("ANDROID_LIBRARY_UI_VERIFIED $state")
    }

    private fun assertExplicitConcealedAndRevealed(view: String) {
        waitUntil("explicit fixture placeholder in $view") {
            evaluate("Boolean(document.querySelector('.cover__reveal button,.row__reveal button'))") == "true"
        }
        val concealed = JSONObject(decoded(evaluate("""
          JSON.stringify((()=>{const b=document.querySelector('.cover__reveal button,.row__reveal button');const item=b.closest('[data-library-game]');window.__vnExplicitItem=item.dataset.libraryGame;return {images:item.querySelectorAll('img').length,buttonHeight:b.getBoundingClientRect().height}})())
        """.trimIndent())))
        assertEquals("Explicit placeholder must not contain an image", 0, concealed.getInt("images"))
        assertTrue("Reveal must be a mobile touch target", concealed.getDouble("buttonHeight") >= 43.5)
        evaluate("document.querySelector('.cover__reveal button,.row__reveal button').click(); null")
        waitUntil("one-time cached reveal in $view") {
            evaluate("(()=>{const e=[...document.querySelectorAll('[data-library-game]')].find(e=>e.dataset.libraryGame===window.__vnExplicitItem);return Boolean(e?.querySelector('img')?.complete && e.querySelector('img').naturalWidth>0 && e.querySelector('img').src.startsWith('data:'))})()") == "true"
        }
        assertTrue("Reveal must stay in library", decoded(evaluate("location.pathname")) in listOf("/", "/index.html"))
        // The following view remount revokes the local Show once state. This
        // avoids changing preferences mid-layout through a test-only store.
    }

    private fun scrollFar(view: String) {
        evaluate(if (view == "shelf") "(()=>{const e=document.querySelector('.library-virtual');e.scrollLeft=50000;e.dispatchEvent(new Event('scroll'));return null})()"
            else "(()=>{const e=document.querySelector('.app__main');e.scrollTop=50000;e.dispatchEvent(new Event('scroll'));return null})()")
        settle()
    }

    private fun reloadLibrary(theme: String) {
        evaluate("window.__vnLibraryReloadMarker=true; location.reload(); null")
        waitUntil("library reload in $theme") {
            evaluate("!window.__vnLibraryReloadMarker && document.documentElement.dataset.theme === ${JSONObject.quote(theme)} && document.querySelector('.library__filters .library__count')?.textContent?.includes('1500') && document.querySelectorAll('[data-library-game]').length > 0") == "true"
        }
        settle()
    }

    private fun invoke(command: String, args: JSONObject = JSONObject()): JSONObject {
        val key = "__vnLibraryVerification"
        evaluate("window.$key=null;window.__TAURI_INTERNALS__.invoke(${JSONObject.quote(command)},$args).then(value=>window.$key={ok:true,value},error=>window.$key={ok:false,error});null")
        var response: JSONObject? = null
        waitUntil("IPC $command") {
            val raw = decoded(evaluate("JSON.stringify(window.$key)"))
            if (raw != "null") response = JSONObject(raw)
            response != null
        }
        assertTrue("IPC $command: $response", response!!.getBoolean("ok"))
        return response!!.getJSONObject("value")
    }

    private fun json(script: String): JSONArray = JSONArray(decoded(evaluate("JSON.stringify($script)")))
    private fun decoded(raw: String): String = (JSONTokener(raw).nextValue() as? String) ?: raw
    private fun evaluate(script: String): String {
        val latch = CountDownLatch(1)
        val result = AtomicReference<String>()
        // Same signature as AndroidTrackingTest; WebView requires the UI thread.
        instrumentation.runOnMainSync {
            webView.evaluateJavascript(script) { result.set(it); latch.countDown() }
        }
        assertTrue("WebView evaluation timed out", latch.await(15, TimeUnit.SECONDS))
        return result.get()
    }

    private fun settle() { Thread.sleep(400) }
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
        if (view is ViewGroup) for (index in 0 until view.childCount) {
            findWebView(view.getChildAt(index))?.let { return it }
        }
        return null
    }
}
