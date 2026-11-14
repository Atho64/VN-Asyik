import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { companionApi, type CompanionSnapshot } from "@/lib/ipc/companion";
import {
  visionApi,
  type VisionCrop,
  type VisionPreview,
} from "@/lib/ipc/companionVision";
import { companionWindowApi } from "@/lib/ipc/companionWindow";
import { acceptSnapshot, failureCode } from "./store";
import { companionError } from "./errors";

const fullCrop: VisionCrop = { x: 0, y: 0, width: 1000, height: 1000 };

export function CompanionVision({
  snapshot,
  overlay,
  onOpen,
}: {
  snapshot: CompanionSnapshot;
  overlay: boolean;
  onOpen?: () => void;
}) {
  const { t } = useI18n();
  const [params, setParams] = useSearchParams();
  const [open, setOpen] = useState(false);
  const [windows, setWindows] = useState<{ id: string; title: string }[]>([]);
  const [selected, setSelected] = useState("");
  const [crop, setCrop] = useState(fullCrop);
  const [preview, setPreview] = useState<VisionPreview | null>(null);
  const previewRef = useRef<VisionPreview | null>(null);
  const previewRequest = useRef(0);
  const [question, setQuestion] = useState("");
  const [consent, setConsent] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const view = snapshot.vision;
  const session = snapshot.session;
  const sessionId = session?.id;
  const dismiss = useCallback(() => {
    previewRequest.current += 1;
    const frame = previewRef.current;
    previewRef.current = null;
    if (frame && sessionId)
      void visionApi
        .control(sessionId, "dismiss", frame.generation, frame.token)
        .catch(() => {});
  }, [sessionId]);
  useEffect(() => () => dismiss(), [dismiss]);
  const requested = !overlay && params.get("vision") === "1";
  const close = useCallback(() => {
    setOpen(false);
    dismiss();
    setPreview(null);
    setConsent(false);
    if (params.has("vision")) {
      const next = new URLSearchParams(params);
      next.delete("vision");
      void setParams(next, { replace: true });
    }
  }, [params, setParams, dismiss]);
  const usable =
    snapshot.lifecycle === "active" &&
    session?.options.knowledge !== "metadata" &&
    snapshot.capabilities.vision === "available";
  const current = preview?.generation === view.generation ? preview : null;
  const validCrop =
    Object.values(crop).every(
      (value) => Number.isInteger(value) && value >= 0 && value <= 1000,
    ) &&
    crop.width > 0 &&
    crop.height > 0 &&
    crop.x + crop.width <= 1000 &&
    crop.y + crop.height <= 1000;

  async function run(action: () => Promise<void>) {
    setPending(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(failureCode(e));
    } finally {
      setPending(false);
    }
  }
  async function controls(action: "quiet" | "stop") {
    if (!session) return;
    dismiss();
    await run(async () => {
      acceptSnapshot(await visionApi.control(session.id, action, view.generation));
      setPreview(null);
      setConsent(false);
    });
  }
  async function showPreview(reselect: boolean) {
    if (!session) return;
    dismiss();
    const request = previewRequest.current;
    setPreview(null);
    setConsent(false);
    await run(async () => {
      const result = reselect
        ? await visionApi.select(session.id, selected, crop)
        : await visionApi.preview(session.id, view.generation);
      if (request !== previewRequest.current) {
        await visionApi.control(session.id, "dismiss", result.generation, result.token);
        return;
      }
      previewRef.current = result;
      acceptSnapshot(await companionApi.snapshot());
      if (request !== previewRequest.current) return;
      setPreview(result);
    });
  }
  if (!session) return null;
  return (
    <div className="companion-vision">
      <div className="companion-vision__bar">
        <Button
          variant="ghost"
          onClick={() => {
            if (overlay) void companionWindowApi.showMain("/companion?vision=1");
            else {
              onOpen?.();
              setOpen(true);
            }
          }}
        >
          {t("vision.title")}
        </Button>
        <span role="status">{t(`vision.state.${view.status}`)}</span>
        {view.watching && (
          <Button
            variant="ghost"
            onClick={() => void controls("quiet")}
            disabled={view.quiet}
          >
            {t("vision.quiet")}
          </Button>
        )}
        {view.window && (
          <Button variant="ghost" onClick={() => void controls("stop")}>
            {t("vision.stop")}
          </Button>
        )}
      </div>
      {(error || view.error) && (
        <p className="companion__error" role="alert">
          {companionError(t, error ?? view.error!)}
        </p>
      )}
      {!overlay && (
        <Sheet open={open || requested} onClose={close} title={t("vision.title")}>
          <div className="companion-vision__panel">
            <p>{t("vision.intro")}</p>
            {!usable && (
              <p role="status">
                {t(
                  snapshot.capabilities.vision !== "available"
                    ? "vision.unsupported"
                    : session.options.knowledge === "metadata"
                      ? "vision.metadata"
                      : "vision.resumeSession",
                )}
              </p>
            )}
            <div className="companion-vision__actions">
              <Button
                disabled={!usable || pending || snapshot.busy}
                onClick={() =>
                  void run(async () => {
                    dismiss();
                    setPreview(null);
                    setWindows(await visionApi.windows());
                    setSelected("");
                  })
                }
              >
                {t("vision.listWindows")}
              </Button>
              {view.window && (
                <Button onClick={() => void controls("stop")}>{t("vision.stop")}</Button>
              )}
            </div>
            <label className="companion__field">
              <span>{t("vision.window")}</span>
              <select
                value={selected}
                disabled={!usable || pending}
                onChange={(e) => {
                  dismiss();
                  setSelected(e.target.value);
                  setPreview(null);
                  setConsent(false);
                }}
              >
                <option value="">{t("vision.chooseWindow")}</option>
                {windows.map((w) => (
                  <option key={w.id} value={w.id}>
                    {w.title}
                  </option>
                ))}
              </select>
            </label>
            <details>
              <summary>{t("vision.crop")}</summary>
              <div className="companion-vision__crop">
                {(["x", "y", "width", "height"] as const).map((key) => (
                  <label className="companion__field" key={key}>
                    <span>{t(`vision.crop.${key}`)}</span>
                    <input
                      type="number"
                      min={key === "width" || key === "height" ? 1 : 0}
                      max={100}
                      step={1}
                      value={crop[key] / 10}
                      onChange={(e) => {
                        dismiss();
                        setCrop({ ...crop, [key]: Number(e.target.value) * 10 });
                        setPreview(null);
                        setConsent(false);
                      }}
                    />
                  </label>
                ))}
              </div>
              <Button
                variant="ghost"
                onClick={() => {
                  dismiss();
                  setCrop(fullCrop);
                  setPreview(null);
                  setConsent(false);
                }}
              >
                {t("vision.resetCrop")}
              </Button>
            </details>
            <Button
              disabled={!usable || !selected || !validCrop || pending || snapshot.busy}
              onClick={() => void showPreview(true)}
            >
              {t("vision.selectPreview")}
            </Button>
            {view.window && (
              <>
                <p className="companion-vision__identity">
                  <strong>{view.window}</strong>
                  <br />
                  {view.provider?.model}
                  <br />
                  {view.provider?.baseUrl}
                </p>
                <p>{t("vision.probeCost")}</p>
                <Button
                  disabled={
                    pending || snapshot.busy || view.support === "testing" || !usable
                  }
                  onClick={() =>
                    void run(async () => {
                      acceptSnapshot(await visionApi.probe(session.id));
                    })
                  }
                >
                  {t("vision.test")}
                </Button>
                <p role="status">
                  {t(view.support === "accepted" ? "vision.accepted" : "vision.unknown")}
                </p>
                <Button
                  disabled={pending || snapshot.busy || !usable}
                  onClick={() => void showPreview(false)}
                >
                  {t("vision.look")}
                </Button>
              </>
            )}
            {current && (
              <>
                <figure className="companion-vision__preview">
                  <img
                    src={current.dataUrl}
                    alt={t("vision.previewAlt")}
                    width={current.width}
                    height={current.height}
                  />
                  <figcaption>
                    {t("vision.previewSize", {
                      width: current.width,
                      height: current.height,
                    })}
                  </figcaption>
                </figure>
                <label className="companion__field">
                  <span>{t("vision.question")}</span>
                  <textarea
                    value={question}
                    maxLength={16000}
                    rows={2}
                    onChange={(e) => setQuestion(e.target.value)}
                  />
                </label>
                <Button
                  disabled={
                    pending || snapshot.busy || !usable || !(view.support === "accepted")
                  }
                  onClick={() =>
                    void run(async () => {
                      acceptSnapshot(
                        await visionApi.send(
                          session.id,
                          current.token,
                          question.trim() || t("vision.defaultQuestion"),
                        ),
                      );
                      previewRef.current = null;
                      setPreview(null);
                      setQuestion("");
                      setConsent(false);
                    })
                  }
                >
                  {t("vision.send")}
                </Button>
                <label className="companion-vision__consent">
                  <input
                    type="checkbox"
                    checked={consent}
                    onChange={(e) => setConsent(e.target.checked)}
                  />
                  <span>{t("vision.consent")}</span>
                </label>
                <Button
                  variant="primary"
                  disabled={
                    !consent ||
                    pending ||
                    snapshot.busy ||
                    !usable ||
                    !(view.support === "accepted")
                  }
                  onClick={() =>
                    void run(async () => {
                      acceptSnapshot(
                        await visionApi.control(
                          session.id,
                          "watch",
                          view.generation,
                          current.token,
                        ),
                      );
                      previewRef.current = null;
                      setPreview(null);
                      setConsent(false);
                      close();
                    })
                  }
                >
                  {t("vision.watch")}
                </Button>
              </>
            )}
            <p>{t("vision.limits", { hourly: view.hourly, total: view.total })}</p>
            <p className="companion__muted">{t("vision.retention")}</p>
            {pending && <p role="status">{t("companion.loading")}</p>}
          </div>
        </Sheet>
      )}
    </div>
  );
}
