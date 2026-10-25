VN Asyik $TAG — pre-release.

## Android

Signed with the release keystore (`CN=VN Asyik`), package
`com.atho.vnasyik`. Android still warns about installing from an unknown
source, because that is what installing outside the Play Store means — not
because the APK is unsigned.

## Windows

The `.msi` and `.exe` are **unsigned**: this project has no code-signing
certificate. Windows SmartScreen will warn, and you will have to click
"More info → Run anyway". That is expected.

## Known limitations

- Chapter detection comes from the window title only. It works for
  SiglusEngine; KiriKiri and Artemis have no automatic source, so chapters
  there are manual.
- No in-app game hooking. Live text comes from LunaTranslator's own network
  service.
- Character traits are hidden above spoiler level 0 until you ask for them.
- Upgrading from a build made before the rename to `com.atho.vnasyik`:
  Android treats it as a different app, so export with
  *Settings → Backup & transfer* in the old build and import here. On Windows,
  copy `%APPDATA%\app.vntracker.desktop\` to `%APPDATA%\com.atho.vnasyik\`.
