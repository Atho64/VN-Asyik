import { useEffect, useMemo, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { api, IpcError } from "@/lib/ipc/client";
import {
  drawShareCard,
  shareCardDataUrl,
  shareCardFilename,
  type ShareCardFormat,
  type ShareCardLabels,
  type ShareCardModel,
} from "./shareCardCanvas";

interface ShareCardProps {
  model: ShareCardModel;
  /** The sheet title, already localized for a year or for all time. */
  title: string;
  open: boolean;
  onClose: () => void;
}

/**
 * A wrapped card rendered as a picture.
 *
 * The canvas is the preview and the export source at once, so what the reader
 * sees is exactly the PNG that gets saved. Nothing leaves the device: the image
 * is generated locally and handed to the platform picker or share sheet.
 */
export function ShareCard({ model, title, open, onClose }: ShareCardProps) {
  const { t, locale } = useI18n();
  const isAndroid = useAppStore((state) => state.platform?.os === "android");
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [format, setFormat] = useState<ShareCardFormat>("post");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const labels = useMemo<ShareCardLabels>(
    () => ({
      totalTime: t("review.totalTime"),
      mostRead: t("review.topGames"),
      footer: t("review.shareFooter"),
      hourTitle: t("review.hourTitle"),
    }),
    [t],
  );

  useEffect(() => {
    if (!open) return undefined;
    const canvas = canvasRef.current;
    if (!canvas) return undefined;
    let cancelled = false;
    setError(null);
    setSaved(false);
    void drawShareCard(canvas, { model, labels, locale, format }).catch(
      (caught: unknown) => {
        if (!cancelled) {
          setError(caught instanceof Error ? caught.message : String(caught));
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [open, model, labels, locale, format]);

  const run = async (action: "save" | "share"): Promise<void> => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const dataUrl = shareCardDataUrl(canvas);
      const filename = shareCardFilename(model, format);
      if (action === "save") {
        const didSave = await api.saveShareCard(dataUrl, filename);
        if (didSave) setSaved(true);
      } else {
        await api.shareShareCard(dataUrl, filename);
      }
    } catch (caught) {
      setError(caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      open={open}
      title={title}
      onClose={onClose}
      footer={
        <div className="share-card__actions">
          {isAndroid ? (
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void run("share")}
            >
              {t("review.shareAction")}
            </Button>
          ) : null}
          <Button
            variant={isAndroid ? "secondary" : "primary"}
            disabled={busy}
            onClick={() => void run("save")}
          >
            {t("review.saveImage")}
          </Button>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            {t("common.close")}
          </Button>
        </div>
      }
    >
      <div className="share-card">
        <div
          className="share-card__formats"
          role="tablist"
          aria-label={t("review.shareFormat")}
        >
          {(["post", "story"] as const).map((option) => (
            <button
              key={option}
              type="button"
              role="tab"
              className="share-card__format"
              aria-selected={format === option}
              data-active={format === option}
              onClick={() => setFormat(option)}
            >
              {option === "post"
                ? t("review.shareFormatPost")
                : t("review.shareFormatStory")}
            </button>
          ))}
        </div>
        <canvas
          ref={canvasRef}
          className={`share-card__canvas share-card__canvas--${format}`}
          role="img"
          aria-label={title}
        />
        <p className="caption share-card__hint">{t("review.shareHint")}</p>
        {saved ? (
          <p className="notice" role="status">
            {t("review.shareSaved")}
          </p>
        ) : null}
        {error ? (
          <p className="field__error" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </Sheet>
  );
}
