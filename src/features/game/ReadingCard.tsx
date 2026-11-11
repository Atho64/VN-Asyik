import { useCallback, useEffect, useId, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { Sheet } from "@/components/Sheet";
import { JournalPhoto } from "@/features/game/JournalPhoto";
import { JournalPhotoPicker } from "@/features/game/JournalPhotoPicker";
import { api, IpcError } from "@/lib/ipc/client";
import {
  readingApi,
  type ReadingContext,
  type ReadingEntry,
  type SaveReadingInput,
  type SessionReadingContext,
} from "@/lib/ipc/reading";
import { formatDateTime, formatDurationLong } from "@/lib/time";

interface ReadingCardProps {
  gameId: string;
  /**
   * The game's VNDB id, when it has one. A pinned photo is a VNDB screenshot, so
   * without a VN there is nothing to pin and the picker says so.
   */
  vnId: string | null;
  /** Game title, used by the photo picker and the export document. */
  title: string;
  onWalkthrough: () => void;
  /** Also refresh after Android has reconciled its foreground events. */
  sessionVersion?: number | undefined;
  selectedSessionId?: string | null;
  onCloseSession?: () => void;
  onEntriesChange?: () => void;
}

const noop = () => undefined;

export function ReadingCard({
  gameId,
  vnId,
  title,
  onWalkthrough,
  sessionVersion,
  selectedSessionId = null,
  onCloseSession = noop,
  onEntriesChange = noop,
}: ReadingCardProps) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);
  const fieldId = useId();
  const [data, setData] = useState<ReadingContext | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [draft, setDraft] = useState<SaveReadingInput | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [deleting, setDeleting] = useState<ReadingEntry | null>(null);
  const [saved, setSaved] = useState(false);
  const request = useRef(0);
  const saving = useRef(false);
  const [sessionData, setSessionData] = useState<SessionReadingContext | null>(null);
  const [sessionLoading, setSessionLoading] = useState(false);
  const [failedSessionId, setFailedSessionId] = useState<string | null>(null);
  const [draftPlayedAt, setDraftPlayedAt] = useState<number | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const sessionRequest = useRef(0);

  const loadSession = useCallback(
    (offset = 0) => {
      if (!selectedSessionId) return Promise.resolve();
      const current = ++sessionRequest.current;
      return readingApi
        .session(gameId, selectedSessionId, offset)
        .then((next) => {
          if (current !== sessionRequest.current) return;
          setFailedSessionId(null);
          setSessionData((previous) => ({
            ...next,
            entries:
              offset === 0
                ? next.entries
                : [
                    ...(previous?.entries ?? []),
                    ...next.entries.filter(
                      (entry) =>
                        !previous?.entries.some((old) => old.meta.id === entry.meta.id),
                    ),
                  ],
          }));
        })
        .catch((caught: unknown) => {
          if (current !== sessionRequest.current) return;
          setFailedSessionId(selectedSessionId);
          if (caught instanceof IpcError && caught.code === "not_found")
            setSessionData(null);
        })
        .finally(() => {
          if (current === sessionRequest.current) setSessionLoading(false);
        });
    },
    [gameId, selectedSessionId],
  );

  useEffect(() => {
    void loadSession();
    const refresh = () => {
      if (selectedSessionId && document.visibilityState !== "hidden" && !saving.current) {
        setSessionLoading(true);
        void loadSession();
      }
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      sessionRequest.current += 1;
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [loadSession, sessionVersion, selectedSessionId]);

  const selected = sessionData?.session.id === selectedSessionId ? sessionData : null;
  const sessionError =
    selectedSessionId !== null && failedSessionId === selectedSessionId;
  const reloadSession = (offset = 0) => {
    setSessionLoading(true);
    void loadSession(offset);
  };

  const load = useCallback(
    (offset = 0) => {
      const current = ++request.current;
      return readingApi
        .context(gameId, offset)
        .then((next) => {
          if (current !== request.current) return;
          setError(false);
          setData((previous) => ({
            ...next,
            entries:
              offset === 0
                ? next.entries
                : [
                    ...(previous?.entries ?? []),
                    ...next.entries.filter(
                      (entry) =>
                        !previous?.entries.some((old) => old.meta.id === entry.meta.id),
                    ),
                  ],
          }));
        })
        .catch(() => {
          if (current === request.current) setError(true);
        })
        .finally(() => {
          if (current === request.current) setLoading(false);
        });
    },
    [gameId],
  );

  useEffect(() => {
    void load();
    const refresh = () => {
      if (document.visibilityState !== "hidden" && !saving.current) {
        setLoading(true);
        void load();
      }
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      request.current += 1;
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [load, sessionVersion]);

  const latest = data?.latestSession;
  const latestEntry = data?.entries[0];
  const reload = (offset = 0) => {
    setLoading(true);
    void load(offset);
  };
  const closeEditor = useCallback(() => {
    if (!saving.current) {
      setDraft(null);
      setFormError(null);
    }
  }, []);
  const closeDelete = useCallback(() => {
    if (!saving.current) {
      setDeleting(null);
      setFormError(null);
    }
  }, []);

  const edit = (entry?: ReadingEntry) => {
    if (!entry && selectedSessionId && (!selected || sessionLoading || sessionError))
      return;
    const target = selectedSessionId ? selected?.session : latest;
    setFormError(null);
    setSaved(false);
    setDraftPlayedAt(entry ? entry.played_at : (target?.endedAt ?? null));
    setDraft({
      id: entry?.meta.id ?? null,
      expected_updated_at: entry?.meta.updated_at ?? null,
      game_id: gameId,
      session_id: entry ? entry.session_id : (target?.id ?? null),
      note: entry?.note ?? "",
      save_slot: entry?.save_slot ?? "",
      route_name: entry?.route_name ?? target?.routeName ?? "",
      chapter_name: entry?.chapter_name ?? target?.chapterName ?? "",
      photo_image_id: entry?.photo_image_id ?? "",
    });
  };

  const exportJournal = async (format: "markdown" | "html") => {
    if (exporting) return;
    setExporting(true);
    try {
      const outcome = await api.journalExport(gameId, format);
      if (outcome.saved) {
        pushToast(
          "success",
          t("reading.exportDone", {
            filename: outcome.filename,
            count: outcome.entries,
          }),
        );
        setExportOpen(false);
      }
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("reading.exportError"),
      );
    } finally {
      setExporting(false);
    }
  };

  const mutate = async (operation: () => Promise<unknown>) => {
    if (saving.current) return;
    saving.current = true;
    setBusy(true);
    setFormError(null);
    try {
      await operation();
      onEntriesChange();
      setDraft(null);
      setDeleting(null);
      setSaved(true);
      if (selectedSessionId) setSessionLoading(true);
      await Promise.all([load(), loadSession()]);
    } catch (caught) {
      if (caught instanceof IpcError && caught.code === "reading_conflict") {
        onEntriesChange();
        await Promise.all([load(), loadSession()]);
      }
      setFormError(
        caught instanceof IpcError && caught.code === "reading_conflict"
          ? t("reading.conflict")
          : t("reading.saveError"),
      );
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  const updateDraft = (
    field: "note" | "save_slot" | "route_name" | "chapter_name",
    value: string,
  ) => setDraft((previous) => (previous ? { ...previous, [field]: value } : null));

  const entryBody = (entry: ReadingEntry) => (
    <>
      {entry.photo_image_id && (
        <JournalPhoto vnId={vnId} imageId={entry.photo_image_id} title={title} />
      )}
      {(entry.route_name || entry.chapter_name || entry.save_slot) && (
        <dl className="reading__position">
          {entry.route_name && (
            <div>
              <dt>{t("reading.route")}</dt>
              <dd>{entry.route_name}</dd>
            </div>
          )}
          {entry.chapter_name && (
            <div>
              <dt>{t("reading.chapter")}</dt>
              <dd>{entry.chapter_name}</dd>
            </div>
          )}
          {entry.save_slot && (
            <div>
              <dt>{t("reading.slot")}</dt>
              <dd>{entry.save_slot}</dd>
            </div>
          )}
        </dl>
      )}
      {entry.note && <p className="reading__note">{entry.note}</p>}
    </>
  );

  const entryActions = (entry: ReadingEntry) => (
    <div className="reading__entry-actions">
      <Button variant="ghost" onClick={() => edit(entry)}>
        {t("reading.edit")}
      </Button>
      <Button
        variant="ghost"
        onClick={() => {
          setFormError(null);
          setDeleting(entry);
        }}
      >
        {t("reading.remove")}
      </Button>
    </div>
  );

  return (
    <>
      <Card className="reading-card">
        <CardHeader
          title={t("reading.title")}
          aside={
            <Button variant="primary" onClick={() => edit()} disabled={!data || loading}>
              <Icon name="plus" />
              {t("reading.add")}
            </Button>
          }
        />
        {loading && !data && (
          <p className="caption" role="status">
            {t("common.loading")}
          </p>
        )}
        {error && (
          <div className="reading__error" role="alert">
            <p>{t("reading.loadError")}</p>
            <Button variant="secondary" onClick={() => reload()} disabled={loading}>
              {t("reading.retry")}
            </Button>
          </div>
        )}
        {data && (
          <>
            <div className="reading__overview">
              <div className="reading__session">
                <p className="caption">{t("reading.lastSession")}</p>
                {latest ? (
                  <>
                    <p className="reading__date">
                      {formatDateTime(latest.endedAt, locale)}
                    </p>
                    <p className="caption">
                      {formatDurationLong(latest.countedMs, locale)}
                    </p>
                    {latest.routeName || latest.chapterName ? (
                      <dl className="reading__position">
                        {latest.routeName && (
                          <div>
                            <dt>{t("reading.route")}</dt>
                            <dd>{latest.routeName}</dd>
                          </div>
                        )}
                        {latest.chapterName && (
                          <div>
                            <dt>{t("reading.chapter")}</dt>
                            <dd>{latest.chapterName}</dd>
                          </div>
                        )}
                      </dl>
                    ) : (
                      <p className="caption reading__hint">{t("reading.noPosition")}</p>
                    )}
                  </>
                ) : (
                  <p className="body">{t("reading.noSession")}</p>
                )}
              </div>
              <div className="reading__checkpoint">
                <p className="caption">{t("reading.lastNote")}</p>
                {latestEntry ? (
                  <>
                    <p className="caption reading__timestamp">
                      {formatDateTime(latestEntry.created_at, locale)}
                    </p>
                    {entryBody(latestEntry)}
                    {entryActions(latestEntry)}
                  </>
                ) : (
                  <p className="body">{t("reading.empty")}</p>
                )}
              </div>
            </div>
            <div className="reading__actions">
              <button type="button" className="reading__guide" onClick={onWalkthrough}>
                {t("reading.guide")}
              </button>
              <Button
                variant="ghost"
                onClick={() => setExportOpen(true)}
                disabled={data.entries.length === 0}
              >
                {t("reading.export")}
              </Button>
              {(data.entries.length > 1 || data.hasMore) && (
                <Button
                  variant="ghost"
                  aria-expanded={historyOpen}
                  onClick={() => setHistoryOpen(!historyOpen)}
                >
                  {historyOpen ? t("reading.hideHistory") : t("reading.history")}
                </Button>
              )}
            </div>
            {saved && (
              <p className="caption" role="status">
                {t("reading.saved")}
              </p>
            )}
            {historyOpen && (
              <div className="reading__history">
                <h3 className="h3">{t("reading.history")}</h3>
                <ol className="reading__entries">
                  {data.entries.slice(1).map((entry) => (
                    <li key={entry.meta.id}>
                      <p className="caption">
                        {formatDateTime(entry.created_at, locale)}
                      </p>
                      {entry.played_at !== null && (
                        <p className="caption">
                          {t("reading.forSession", {
                            date: formatDateTime(entry.played_at, locale),
                          })}
                        </p>
                      )}
                      {entryBody(entry)}
                      {entryActions(entry)}
                    </li>
                  ))}
                </ol>
                {data.hasMore && (
                  <Button
                    variant="secondary"
                    disabled={loading}
                    onClick={() => reload(data.entries.length)}
                  >
                    {t("reading.more")}
                  </Button>
                )}
              </div>
            )}
          </>
        )}
      </Card>

      <Sheet
        open={selectedSessionId !== null && !draft && !deleting}
        title={t("reading.sessionDetails")}
        onClose={onCloseSession}
      >
        <div className="reading__session-detail">
          {!selected && (!sessionError || sessionLoading) && (
            <p role="status">{t("common.loading")}</p>
          )}
          {sessionError && (
            <div className="reading__error" role="alert">
              <p>{t("reading.sessionError")}</p>
              <Button onClick={() => reloadSession()} disabled={sessionLoading}>
                {t("reading.retry")}
              </Button>
            </div>
          )}
          {selected && (
            <>
              <dl className="reading__session-facts">
                <div>
                  <dt>{t("reading.started")}</dt>
                  <dd>{formatDateTime(selected.session.startedAt, locale)}</dd>
                </div>
                <div>
                  <dt>{t("reading.ended")}</dt>
                  <dd>{formatDateTime(selected.session.endedAt, locale)}</dd>
                </div>
                <div>
                  <dt>{t("reading.readingTime")}</dt>
                  <dd>{formatDurationLong(selected.session.countedMs, locale)}</dd>
                </div>
                <div>
                  <dt>{t("detail.away")}</dt>
                  <dd>{formatDurationLong(selected.session.afkMs, locale)}</dd>
                </div>
                {selected.session.routeName && (
                  <div>
                    <dt>{t("reading.route")}</dt>
                    <dd>{selected.session.routeName}</dd>
                  </div>
                )}
                {selected.session.chapterName && (
                  <div>
                    <dt>{t("reading.chapter")}</dt>
                    <dd>{selected.session.chapterName}</dd>
                  </div>
                )}
              </dl>
              {!selected.session.routeName && !selected.session.chapterName && (
                <p className="caption">{t("reading.noPosition")}</p>
              )}
              <div className="reading__session-heading">
                <h3 className="h3">{t("reading.sessionNotes")}</h3>
                <Button
                  variant="primary"
                  onClick={() => edit()}
                  disabled={sessionLoading || sessionError}
                >
                  <Icon name="plus" />
                  {t("reading.add")}
                </Button>
              </div>
              {saved && (
                <p className="caption" role="status">
                  {t("reading.saved")}
                </p>
              )}
              {selected.entries.length === 0 && (
                <p className="body">{t("reading.sessionEmpty")}</p>
              )}
              <ol className="reading__entries">
                {selected.entries.map((entry) => (
                  <li key={entry.meta.id}>
                    <p className="caption">{formatDateTime(entry.created_at, locale)}</p>
                    {entryBody(entry)}
                    {entryActions(entry)}
                  </li>
                ))}
              </ol>
              {selected.hasMore && (
                <Button
                  disabled={sessionLoading}
                  onClick={() => reloadSession(selected.entries.length)}
                >
                  {t("reading.more")}
                </Button>
              )}
            </>
          )}
        </div>
      </Sheet>

      <Sheet
        open={draft !== null}
        title={draft?.id ? t("reading.edit") : t("reading.add")}
        onClose={closeEditor}
      >
        {draft && (
          <form
            className="reading__form"
            onSubmit={(event) => {
              event.preventDefault();
              void mutate(() => readingApi.save(draft));
            }}
          >
            <p className="caption">{t("reading.manualHint")}</p>
            <div className="reading__fields">
              {(["route_name", "chapter_name", "save_slot"] as const).map((field) => (
                <label className="field" key={field} htmlFor={`${fieldId}-${field}`}>
                  <span className="field__label">
                    {t(
                      field === "route_name"
                        ? "reading.route"
                        : field === "chapter_name"
                          ? "reading.chapter"
                          : "reading.slot",
                    )}
                  </span>
                  <input
                    className="input"
                    id={`${fieldId}-${field}`}
                    value={draft[field]}
                    maxLength={field === "save_slot" ? 80 : 200}
                    disabled={busy}
                    onChange={(event) => updateDraft(field, event.target.value)}
                  />
                </label>
              ))}
            </div>
            <div className="reading__photo-field">
              <p className="field__label">{t("reading.photo")}</p>
              {draft.photo_image_id ? (
                <div className="reading__photo-chosen">
                  <JournalPhoto
                    vnId={vnId}
                    imageId={draft.photo_image_id}
                    title={title}
                  />
                  <div className="reading__photo-actions">
                    <Button
                      variant="ghost"
                      disabled={busy}
                      onClick={() => setPickerOpen(true)}
                    >
                      {t("reading.photoChange")}
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={busy}
                      onClick={() =>
                        setDraft((previous) =>
                          previous ? { ...previous, photo_image_id: "" } : null,
                        )
                      }
                    >
                      {t("reading.photoRemove")}
                    </Button>
                  </div>
                </div>
              ) : (
                <Button
                  variant="secondary"
                  disabled={busy || !vnId}
                  onClick={() => setPickerOpen(true)}
                >
                  <Icon name="image" />
                  {t("reading.photoPick")}
                </Button>
              )}
            </div>
            <label className="field" htmlFor={`${fieldId}-note`}>
              <span className="field__label">{t("reading.note")}</span>
              <textarea
                className="textarea"
                id={`${fieldId}-note`}
                value={draft.note}
                maxLength={4000}
                rows={5}
                disabled={busy}
                placeholder={t("reading.placeholder")}
                onChange={(event) => updateDraft("note", event.target.value)}
              />
            </label>
            <p className="caption">
              {draft.session_id ? t("reading.linkedSession") : t("reading.manualEntry")}
            </p>
            {draftPlayedAt !== null && (
              <p className="caption">
                {t("reading.forSession", { date: formatDateTime(draftPlayedAt, locale) })}
              </p>
            )}
            {formError && (
              <p className="notice notice--danger" role="alert">
                {formError}
              </p>
            )}
            <div className="reading__form-actions">
              <Button variant="ghost" onClick={closeEditor} disabled={busy}>
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  busy ||
                  !draft.photo_image_id.trim() &&
                    ![
                      draft.note,
                      draft.save_slot,
                      draft.route_name,
                      draft.chapter_name,
                    ].some((text) => text.trim())
                }
              >
                {busy ? t("reading.saving") : t("reading.save")}
              </Button>
            </div>
          </form>
        )}
      </Sheet>

      <Sheet
        open={pickerOpen}
        title={t("reading.photoPick")}
        onClose={() => setPickerOpen(false)}
        wide
      >
        <JournalPhotoPicker
          vnId={vnId}
          title={title}
          selected={draft?.photo_image_id ?? ""}
          onSelect={(imageId) => {
            setDraft((previous) =>
              previous ? { ...previous, photo_image_id: imageId } : null,
            );
            setPickerOpen(false);
          }}
        />
      </Sheet>

      <Sheet
        open={exportOpen}
        title={t("reading.export")}
        onClose={() => {
          if (!exporting) setExportOpen(false);
        }}
      >
        <p className="body">{t("reading.exportHint")}</p>
        <div className="reading__export-choices">
          <Button
            variant="primary"
            disabled={exporting}
            onClick={() => void exportJournal("markdown")}
          >
            {t("reading.exportMarkdown")}
          </Button>
          <Button
            variant="secondary"
            disabled={exporting}
            onClick={() => void exportJournal("html")}
          >
            {t("reading.exportHtml")}
          </Button>
        </div>
        <p className="caption">{t("reading.exportFormats")}</p>
      </Sheet>

      <Sheet open={deleting !== null} title={t("reading.remove")} onClose={closeDelete}>
        <p className="body">{t("reading.removeConfirm")}</p>
        {formError && (
          <p className="notice notice--danger" role="alert">
            {formError}
          </p>
        )}
        <div className="reading__form-actions">
          <Button variant="ghost" onClick={closeDelete} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() => {
              if (deleting) void mutate(() => readingApi.remove(gameId, deleting));
            }}
          >
            {t("reading.remove")}
          </Button>
        </div>
      </Sheet>
    </>
  );
}
