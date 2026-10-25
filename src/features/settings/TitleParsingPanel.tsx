import { useEffect, useState } from "react";

import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { api, IpcError } from "@/lib/ipc/client";
import type { PatternTest, TitleConfig } from "@/lib/ipc/types";

/**
 * How chapters are read from window titles.
 *
 * Siglus writes its progress into the caption and that format is verified, so it
 * is the default. Other engines may do the same in formats nobody here has seen,
 * so an advanced pattern covers them without waiting for a build.
 *
 * The tester is not decoration: a pattern that never matches looks exactly like
 * a game that writes no titles, and only showing the match makes that
 * distinguishable.
 */
export function TitleParsingPanel() {
  const { t } = useI18n();
  const [config, setConfig] = useState<TitleConfig | null>(null);
  const [format, setFormat] = useState("siglus");
  const [pattern, setPattern] = useState("");
  const [sample, setSample] = useState("");
  const [results, setResults] = useState<PatternTest[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    void api
      .trackingTitleConfig()
      .then((next) => {
        if (!active) return;
        setConfig(next);
        setFormat(next.format);
        setPattern(next.pattern ?? "");
        setSample(next.sample ?? "");
      })
      .catch(() => {
        // Optional feature; a failure here must not break Settings.
      });
    return () => {
      active = false;
    };
  }, []);

  async function runTest() {
    setError(null);
    setResults(null);
    try {
      const titles = sample.trim().length > 0 ? [sample] : [];
      setResults(await api.trackingTestTitlePattern(pattern, titles));
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const next = await api.trackingSaveTitleConfig(
        format,
        format === "custom" ? pattern : null,
      );
      setConfig(next);
      setSaved(true);
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  }

  if (!config) return null;

  return (
    <Card>
      <CardHeader title={t("titles.section")} />
      <p className="settings__hint">{t("titles.intro")}</p>

      <label className="field">
        <span className="field__label">{t("titles.format")}</span>
        <select
          className="select"
          value={format}
          onChange={(event) => {
            setFormat(event.target.value);
            setResults(null);
          }}
        >
          <option value="siglus">{t("titles.siglus")}</option>
          <option value="custom">{t("titles.custom")}</option>
          <option value="off">{t("titles.off")}</option>
        </select>
        <span className="field__hint">
          {format === "siglus"
            ? t("titles.siglusHint")
            : format === "off"
              ? t("titles.offHint")
              : t("titles.customHint")}
        </span>
      </label>

      {format === "custom" ? (
        <div style={{ marginTop: "var(--space-3)" }}>
          <label className="field">
            <span className="field__label">{t("titles.pattern")}</span>
            <input
              className="input"
              value={pattern}
              onChange={(event) => {
                setPattern(event.target.value);
                setResults(null);
              }}
              placeholder="^(.*?) - (.+)$"
              spellCheck={false}
            />
            <span className="field__hint">{t("titles.patternHint")}</span>
          </label>

          <label className="field" style={{ marginTop: "var(--space-3)" }}>
            <span className="field__label">{t("titles.sample")}</span>
            <input
              className="input"
              value={sample}
              onChange={(event) => {
                setSample(event.target.value);
                setResults(null);
              }}
              placeholder={t("titles.samplePlaceholder")}
              spellCheck={false}
            />
            <span className="field__hint">{t("titles.sampleHint")}</span>
          </label>

          <div className="chapter-head__actions" style={{ marginTop: "var(--space-2)" }}>
            <Button variant="secondary" onClick={() => void runTest()}>
              {t("titles.test")}
            </Button>
          </div>

          {results?.map((result) => (
            <p key={result.title} className="settings__hint">
              {result.matched ? (
                <>
                  {t("titles.matched")}{" "}
                  <strong>
                    {result.route ? `${result.route} · ` : ""}
                    {result.position}
                  </strong>
                </>
              ) : (
                t("titles.noMatch")
              )}
            </p>
          ))}
        </div>
      ) : null}

      <div className="chapter-head__actions" style={{ marginTop: "var(--space-3)" }}>
        <Button variant="primary" onClick={() => void save()} disabled={busy}>
          {busy ? t("play.saving") : t("play.save")}
        </Button>
      </div>

      {saved && <p className="settings__hint">{t("titles.saved")}</p>}
      {error && <p className="notice notice--danger">{error}</p>}
    </Card>
  );
}
