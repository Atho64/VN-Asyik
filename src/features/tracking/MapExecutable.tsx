import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import type { GameCard, RunnerKind } from "@/lib/ipc/types";

const RUNNER_DEFAULTS: Record<
  RunnerKind,
  { exe: string; androidPkg: string; pkgPrefixes: string[]; targetPlaceholder: string }
> = {
  vita3k: {
    exe: "Vita3K.exe",
    androidPkg: "org.vita3k.emulator",
    pkgPrefixes: ["org.vita3k"],
    targetPlaceholder: "e.g. PCSG00001 or Title ID / ROM path",
  },
  rpcs3: {
    exe: "rpcs3.exe",
    androidPkg: "rpcs3.exe",
    pkgPrefixes: [],
    targetPlaceholder: "e.g. BLUS12345 or game folder / EBOOT.BIN",
  },
  pcsx2: {
    exe: "pcsx2-qtx64.exe",
    androidPkg: "pcsx2-qtx64.exe",
    pkgPrefixes: [],
    targetPlaceholder: "e.g. C:/Games/game.iso",
  },
  eden: {
    exe: "Eden.exe",
    androidPkg: "dev.eden.eden_emulator",
    pkgPrefixes: ["dev.eden", "org.yuzu", "org.suyu", "org.sudachi"],
    targetPlaceholder: "e.g. C:/Games/game.nsp or .xci",
  },
  citron: {
    exe: "Citron.exe",
    androidPkg: "org.citron.citron_emu",
    pkgPrefixes: ["org.citron"],
    targetPlaceholder: "e.g. C:/Games/game.nsp or .xci",
  },
  shadps4: {
    exe: "shadPS4.exe",
    androidPkg: "shadPS4.exe",
    pkgPrefixes: [],
    targetPlaceholder: "e.g. CUSA00001 or game pkg / folder",
  },
  ppsspp: {
    exe: "PPSSPPWindows64.exe",
    androidPkg: "org.ppsspp.ppsspp",
    pkgPrefixes: ["org.ppsspp"],
    targetPlaceholder: "e.g. C:/Games/game.iso",
  },
  kirikiroid2: {
    exe: "org.tvp.kirikiri2",
    androidPkg: "org.tvp.kirikiri2",
    pkgPrefixes: ["org.tvp.kirikiri", "cx.my.kirikiroid"],
    targetPlaceholder: "data.xp3 or game folder path",
  },
  tyranor: {
    exe: "com.pseudogram.tyranor",
    androidPkg: "com.pseudogram.tyranor",
    pkgPrefixes: ["com.pseudogram.tyranor", "com.tyranor"],
    targetPlaceholder: "game folder path",
  },
  onscripter: {
    exe: "com.yuri.onscripter",
    androidPkg: "com.yuri.onscripter",
    pkgPrefixes: ["com.yuri.onscripter", "cn.natdon.onscripter", "com.onscripter"],
    targetPlaceholder: "0.txt / nscript.dat folder path",
  },
  artemis: {
    exe: "com.artemis.engine",
    androidPkg: "com.artemis.engine",
    pkgPrefixes: ["com.artemis", "jp.co.imel", "jp.co.ENTERGRAM", "jp.co.prototype"],
    targetPlaceholder: "root.pfs or game folder path",
  },
  winlator: {
    exe: "com.winlator",
    androidPkg: "com.winlator",
    pkgPrefixes: ["com.winlator", "app.gamenative"],
    targetPlaceholder: "D:\\Games\\Game.exe or shortcut path",
  },
  gamehub: {
    exe: "com.xiaoji.egggame",
    androidPkg: "com.xiaoji.egggame",
    pkgPrefixes: ["com.xiaoji", "com.gamehub"],
    targetPlaceholder: "Container game or exe path",
  },
  native_apk: {
    exe: "com.example.game",
    androidPkg: "com.example.game",
    pkgPrefixes: [],
    targetPlaceholder: "App package name",
  },
  custom: {
    exe: "",
    androidPkg: "",
    pkgPrefixes: [],
    targetPlaceholder: "File path, ROM or launch arguments",
  },
};

/**
 * Point at a game's executable directly or map an emulator / runner target.
 *
 * Supports native PC game executables as well as multi-game emulators and
 * runners (Vita3K, RPCS3, PCSX2, Eden, Citron Neo, shadPS4, PPSSPP,
 * Kirikiroid2, Tyranor, ONScripter, Artemis, Winlator, GameHub, and Native APK).
 */
export function MapExecutable({
  games,
  onMapped,
  fixedGameId,
}: {
  games: GameCard[];
  onMapped: () => Promise<void>;
  /** When set, the game is already chosen and no picker is shown. */
  fixedGameId?: string;
}) {
  const { t } = useI18n();
  const platform = useAppStore((state) => state.platform);
  const isAndroid = platform?.os === "android";

  const [mode, setMode] = useState<"native" | "runner">(isAndroid ? "runner" : "native");
  const [gameId, setGameId] = useState(fixedGameId ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);

  // Runner state
  const defaultRunner: RunnerKind = isAndroid ? "kirikiroid2" : "vita3k";
  const [runnerKind, setRunnerKind] = useState<RunnerKind>(defaultRunner);
  const [runnerExe, setRunnerExe] = useState(
    isAndroid
      ? RUNNER_DEFAULTS[defaultRunner].androidPkg
      : RUNNER_DEFAULTS[defaultRunner].exe,
  );
  const [launchTarget, setLaunchTarget] = useState("");
  const [titleMatch, setTitleMatch] = useState("");
  const [extraArgs, setExtraArgs] = useState("");

  const currentGameTitle =
    games.find((c) => c.game.meta.id === (fixedGameId ?? gameId))?.game.title ?? "";

  async function pickNative() {
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        title: t("tracking.pickTitle"),
        filters: [
          {
            name: "Executable",
            extensions: ["exe", "EXE"],
          },
        ],
      });
      if (typeof chosen !== "string") {
        return;
      }
      const targetId = fixedGameId ?? gameId;
      if (targetId.length === 0) {
        setError(t("tracking.pickNeedsGame"));
        return;
      }
      await api.trackingMapExecutable(targetId, chosen);
      const parts = chosen.split(/[\\/]/);
      setDone(parts[parts.length - 1] ?? chosen);
      await onMapped();
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  async function pickRunnerBinary() {
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        title: t("tracking.browseEmulator"),
        filters: [
          {
            name: "Executable",
            extensions: ["exe", "EXE"],
          },
        ],
      });
      if (typeof chosen === "string") {
        setRunnerExe(chosen);
      }
    } catch {
      // Ignored
    }
  }

  async function pickLaunchTarget() {
    try {
      const chosen = await open({
        multiple: false,
        directory: false,
        title: t("tracking.browseTarget"),
        filters: [
          {
            name: "Game File / ROM / Disc",
            extensions: [
              "iso",
              "ISO",
              "nsp",
              "xci",
              "pkg",
              "bin",
              "xp3",
              "exe",
              "zip",
              "*",
            ],
          },
        ],
      });
      if (typeof chosen === "string") {
        setLaunchTarget(chosen);
      }
    } catch {
      // Ignored
    }
  }

  async function saveRunner() {
    const targetId = fixedGameId ?? gameId;
    if (targetId.length === 0) {
      setError(t("tracking.pickNeedsGame"));
      return;
    }
    if (runnerExe.trim().length === 0) {
      return;
    }

    setBusy(true);
    setError(null);
    setDone(null);
    try {
      const isAndroidPkg = isAndroid;
      await api.trackingMapRunnerTarget({
        gameId: targetId,
        runnerKind,
        exePath: isAndroidPkg ? null : runnerExe.trim(),
        package: isAndroidPkg ? runnerExe.trim() : null,
        launchTarget: launchTarget.trim().length > 0 ? launchTarget.trim() : null,
        titleMatch: titleMatch.trim().length > 0 ? titleMatch.trim() : null,
        extraArgs: extraArgs.trim().length > 0 ? extraArgs.trim() : null,
      });

      setDone(
        t("tracking.runnerMapped", {
          runner: runnerKind,
          game: currentGameTitle || targetId,
        }),
      );
      await onMapped();
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  const effectiveMode = isAndroid ? "runner" : mode;

  if (isAndroid) {
    return (
      <AndroidRunnerMapping
        games={games}
        onMapped={onMapped}
        {...(fixedGameId ? { fixedGameId } : {})}
      />
    );
  }

  return (
    <div className="map-exe">
      {!isAndroid ? (
        <div
          style={{
            display: "flex",
            gap: "var(--space-2)",
            marginBottom: "var(--space-3)",
          }}
        >
          <Chip selected={mode === "native"} onClick={() => setMode("native")}>
            {t("tracking.modeNative")}
          </Chip>
          <Chip selected={mode === "runner"} onClick={() => setMode("runner")}>
            {t("tracking.modeRunner")}
          </Chip>
        </div>
      ) : null}

      {fixedGameId ? null : (
        <label className="field" style={{ marginBottom: "var(--space-3)" }}>
          <span className="field__label">{t("tracking.pickGame")}</span>
          <select
            className="select"
            value={gameId}
            onChange={(event) => {
              const selectedId = event.target.value;
              setGameId(selectedId);
              const card = games.find((c) => c.game.meta.id === selectedId);
              if (card && !titleMatch) {
                setTitleMatch(card.game.title);
              }
            }}
          >
            <option value="">{t("tracking.pickChoose")}</option>
            {games.map((card) => (
              <option key={card.game.meta.id} value={card.game.meta.id}>
                {card.game.title}
              </option>
            ))}
          </select>
        </label>
      )}

      {effectiveMode === "native" ? (
        <Button variant="secondary" onClick={() => void pickNative()} disabled={busy}>
          <Icon name="plus" />
          {busy ? t("tracking.picking") : t("tracking.pickExe")}
        </Button>
      ) : (
        <div style={{ display: "grid", gap: "var(--space-3)" }}>
          <label className="field">
            <span className="field__label">{t("tracking.runner")}</span>
            <select
              className="select"
              value={runnerKind}
              onChange={(event) => {
                const nextKind = event.target.value as RunnerKind;
                setRunnerKind(nextKind);
                const def = RUNNER_DEFAULTS[nextKind];
                if (
                  def &&
                  (!runnerExe ||
                    Object.values(RUNNER_DEFAULTS).some(
                      (d) => d.exe === runnerExe || d.androidPkg === runnerExe,
                    ))
                ) {
                  setRunnerExe(isAndroid ? def.androidPkg : def.exe);
                }
              }}
            >
              <option value="vita3k">{t("tracking.runnerVita3k")}</option>
              <option value="rpcs3">{t("tracking.runnerRpcs3")}</option>
              <option value="pcsx2">{t("tracking.runnerPcsx2")}</option>
              <option value="eden">{t("tracking.runnerEden")}</option>
              <option value="citron">{t("tracking.runnerCitron")}</option>
              <option value="shadps4">{t("tracking.runnerShadps4")}</option>
              <option value="ppsspp">{t("tracking.runnerPpsspp")}</option>
              <option value="custom">{t("tracking.runnerCustom")}</option>
            </select>
          </label>

          <label className="field">
            <span className="field__label">{t("tracking.runnerExeOrPackage")}</span>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <input
                className="input"
                style={{ flex: 1 }}
                value={runnerExe}
                onChange={(e) => setRunnerExe(e.target.value)}
                placeholder={RUNNER_DEFAULTS[runnerKind]?.exe ?? "Vita3K.exe"}
              />
              <Button
                variant="secondary"
                onClick={() => void pickRunnerBinary()}
                disabled={busy}
              >
                {t("tracking.browseEmulator")}
              </Button>
            </div>
            <span className="field__hint">{t("tracking.runnerExeOrPackageHint")}</span>
          </label>

          <label className="field">
            <span className="field__label">{t("tracking.launchTarget")}</span>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <input
                className="input"
                style={{ flex: 1 }}
                value={launchTarget}
                onChange={(e) => setLaunchTarget(e.target.value)}
                placeholder={RUNNER_DEFAULTS[runnerKind]?.targetPlaceholder ?? ""}
              />
              <Button
                variant="secondary"
                onClick={() => void pickLaunchTarget()}
                disabled={busy}
              >
                {t("tracking.browseTarget")}
              </Button>
            </div>
            <span className="field__hint">{t("tracking.launchTargetHint")}</span>
          </label>

          <label className="field">
            <span className="field__label">{t("tracking.titleMatch")}</span>
            <input
              className="input"
              value={titleMatch}
              onChange={(e) => setTitleMatch(e.target.value)}
              placeholder={currentGameTitle || "Window title keyword"}
            />
            <span className="field__hint">{t("tracking.titleMatchHint")}</span>
          </label>

          <label className="field">
            <span className="field__label">{t("tracking.extraArgs")}</span>
            <input
              className="input"
              value={extraArgs}
              onChange={(e) => setExtraArgs(e.target.value)}
              placeholder="e.g. --fullscreen"
            />
          </label>

          <div>
            <Button
              variant="primary"
              onClick={() => void saveRunner()}
              disabled={busy || !runnerExe.trim()}
            >
              <Icon name="check" />
              {busy ? t("tracking.picking") : t("tracking.mapRunner")}
            </Button>
          </div>
        </div>
      )}

      {done && (
        <p className="settings__hint" style={{ marginTop: "var(--space-2)" }}>
          {t("tracking.picked", { file: done })}
        </p>
      )}
      {error && (
        <p className="notice notice--danger" style={{ marginTop: "var(--space-2)" }}>
          {error}
        </p>
      )}
    </div>
  );
}

function AndroidRunnerMapping({
  games,
  onMapped,
  fixedGameId,
}: {
  games: GameCard[];
  onMapped: () => Promise<void>;
  fixedGameId?: string;
}) {
  const { t } = useI18n();
  const [apps, setApps] = useState<Array<{ package: string; label: string }>>([]);
  const [loading, setLoading] = useState(true);
  const [gameId, setGameId] = useState(fixedGameId ?? "");
  const [runnerKind, setRunnerKind] = useState<RunnerKind>("custom");
  const [packageName, setPackageName] = useState("");
  const [launchTarget, setLaunchTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    void api
      .trackingAndroidApps()
      .then((next) => {
        if (active) setApps(next);
      })
      .catch((caught: unknown) => {
        if (active) {
          setError(
            caught instanceof IpcError ? caught.message : t("tracking.appsFailed"),
          );
        }
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [t]);

  const save = async (): Promise<void> => {
    const targetId = fixedGameId ?? gameId;
    if (!targetId || !packageName.trim()) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await api.trackingMapRunnerTarget({
        gameId: targetId,
        runnerKind,
        exePath: null,
        package: packageName.trim(),
        launchTarget: launchTarget.trim().length > 0 ? launchTarget.trim() : null,
        titleMatch: null,
        extraArgs: null,
      });
      await onMapped();
      setSaved(true);
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="map-exe map-exe--android">
      <p className="settings__hint">{t("tracking.androidMapHint")}</p>
      {!fixedGameId ? (
        <label className="field">
          <span className="field__label">{t("tracking.pickGame")}</span>
          <select
            className="select"
            value={gameId}
            onChange={(event) => {
              setGameId(event.target.value);
              setSaved(false);
            }}
          >
            <option value="">{t("tracking.pickChoose")}</option>
            {games.map((card) => (
              <option key={card.game.meta.id} value={card.game.meta.id}>
                {card.game.title}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <label className="field">
        <span className="field__label">{t("tracking.installedApp")}</span>
        <select
          className="select"
          value={apps.some((app) => app.package === packageName) ? packageName : ""}
          disabled={loading || busy}
          onChange={(event) => {
            setPackageName(event.target.value);
            setSaved(false);
          }}
        >
          <option value="">
            {loading ? t("common.loading") : t("tracking.chooseApp")}
          </option>
          {apps.map((app) => (
            <option key={app.package} value={app.package}>
              {app.label} · {app.package}
            </option>
          ))}
        </select>
        {!loading && apps.length === 0 ? (
          <span className="field__hint">{t("tracking.appsEmpty")}</span>
        ) : null}
      </label>
      <details className="map-exe__advanced">
        <summary>{t("tracking.manualPackage")}</summary>
        <div style={{ display: "grid", gap: "var(--space-2)", marginTop: "var(--space-2)" }}>
          <label className="field">
            <span className="field__label">{t("tracking.runner")}</span>
            <select
              className="select"
              value={runnerKind}
              onChange={(event) => {
                const nextKind = event.target.value as RunnerKind;
                setRunnerKind(nextKind);
                setSaved(false);
                const def = RUNNER_DEFAULTS[nextKind];
                if (def && nextKind !== "custom") {
                  const matchedInstalled = apps.find((app) =>
                    def.pkgPrefixes.some((prefix) =>
                      app.package.toLowerCase().startsWith(prefix.toLowerCase()),
                    ),
                  );
                  if (matchedInstalled) {
                    setPackageName(matchedInstalled.package);
                  } else if (!packageName && def.androidPkg) {
                    setPackageName(def.androidPkg);
                  }
                }
              }}
            >
              <option value="custom">{t("tracking.runnerCustom")}</option>
              <option value="kirikiroid2">{t("tracking.runnerKirikiroid2")}</option>
              <option value="tyranor">{t("tracking.runnerTyranor")}</option>
              <option value="onscripter">{t("tracking.runnerOnscripter")}</option>
              <option value="artemis">{t("tracking.runnerArtemis")}</option>
              <option value="winlator">{t("tracking.runnerWinlator")}</option>
              <option value="gamehub">{t("tracking.runnerGamehub")}</option>
              <option value="vita3k">{t("tracking.runnerVita3k")}</option>
              <option value="ppsspp">{t("tracking.runnerPpsspp")}</option>
              <option value="eden">{t("tracking.runnerEden")}</option>
              <option value="citron">{t("tracking.runnerCitron")}</option>
              <option value="native_apk">{t("tracking.runnerNativeApk")}</option>
            </select>
          </label>
          <label className="field">
            <span className="field__label">{t("tracking.packageName")}</span>
            <input
              className="input"
              value={packageName}
              onChange={(event) => {
                setPackageName(event.target.value);
                setSaved(false);
              }}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              placeholder={RUNNER_DEFAULTS[runnerKind]?.androidPkg || "com.example.game"}
            />
            <span className="field__hint">{t("tracking.manualPackageHint")}</span>
          </label>
          <label className="field">
            <span className="field__label">{t("tracking.launchTarget")}</span>
            <input
              className="input"
              value={launchTarget}
              onChange={(event) => {
                setLaunchTarget(event.target.value);
                setSaved(false);
              }}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              placeholder={RUNNER_DEFAULTS[runnerKind]?.targetPlaceholder ?? ""}
            />
            <span className="field__hint">{t("tracking.launchTargetHint")}</span>
          </label>
        </div>
      </details>
      <Button
        variant="primary"
        onClick={() => void save()}
        disabled={busy || !(fixedGameId ?? gameId) || !packageName.trim()}
      >
        <Icon name="check" />
        {busy ? t("tracking.picking") : t("tracking.linkApp")}
      </Button>
      {saved ? (
        <p className="settings__hint" role="status">
          {t("tracking.androidMapped")}
        </p>
      ) : null}
      {error ? (
        <p className="notice notice--danger" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
