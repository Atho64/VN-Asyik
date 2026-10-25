package app.vntracker.desktop

import android.graphics.Color
import android.os.Bundle
import android.view.ViewGroup
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.updatePadding

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    val barColor = Color.parseColor("#101719")
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(barColor),
      navigationBarStyle = SystemBarStyle.dark(barColor),
    )
    super.onCreate(savedInstanceState)

    val rootView = findViewById<ViewGroup>(android.R.id.content)
    rootView.setBackgroundColor(barColor)
    ViewCompat.setOnApplyWindowInsetsListener(rootView) { view, windowInsets ->
      val insets = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      view.updatePadding(
        left = insets.left,
        top = insets.top,
        right = insets.right,
        bottom = insets.bottom,
      )
      WindowInsetsCompat.CONSUMED
    }
  }
}
