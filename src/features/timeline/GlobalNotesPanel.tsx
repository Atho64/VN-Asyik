import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { IpcError } from "@/lib/ipc/client";
import { readingApi, type GlobalNoteView } from "@/lib/ipc/reading";
import { formatDate } from "@/lib/time";
import { requestAchievementsRefresh } from "@/lib/ipc/achievements";
import "@/styles/reading.css";

interface NoteDraft {
  id: string | null;
  expectedUpdatedAt: number | null;
  gameId: string;
  title: string;
  body: string;
  pinned: boolean;
}

const emptyDraft = (): NoteDraft => ({
  id: null,
  expectedUpdatedAt: null,
  gameId: "",
  title: "",
  body: "",
  pinned: false,
});

/**
 * Global and cross-game notebook inside the Timeline Notes tab.
 *
 * Supports optional titles, optional VN links, pinning to the top, searching,
 * and full backup/sync convergence.
 */
export function GlobalNotesPanel() {
  const { t, locale } = useI18n();
  const cards = useAppStore((state) => state.cards);
  const pushToast = useAppStore((state) => state.pushToast);

  const [notes, setNotes] = useState<GlobalNoteView[] | null>(null);
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<string>("all");
  const [editing, setEditing] = useState<NoteDraft | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async (): Promise<void> => {
    try {
      const globalOnly = scope === "global";
      const gameId = scope !== "all" && scope !== "global" ? scope : null;
      const list = await readingApi.listGlobalNotes({
        query: query.trim() || undefined,
        gameId,
        globalOnly,
      });
      setNotes(list);
      setLoadError(null);
    } catch (caught) {
      setLoadError(caught instanceof IpcError ? caught.message : t("common.error"));
    }
  }, [query, scope, t]);

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const globalOnly = scope === "global";
          const gameId = scope !== "all" && scope !== "global" ? scope : null;
          const list = await readingApi.listGlobalNotes({
            query: query.trim() || undefined,
            gameId,
            globalOnly,
          });
          if (!cancelled) {
            setNotes(list);
            setLoadError(null);
          }
        } catch (caught) {
          if (!cancelled) {
            setLoadError(
              caught instanceof IpcError ? caught.message : t("common.error"),
            );
          }
        }
      })();
    }, query.trim() ? 200 : 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, scope, t]);

  const submit = async (event: React.FormEvent): Promise<void> => {
    event.preventDefault();
    if (!editing || busy) return;
    if (!editing.title.trim() && !editing.body.trim()) return;
    setBusy(true);
    try {
      await readingApi.saveGlobalNote({
        id: editing.id,
        expectedUpdatedAt: editing.expectedUpdatedAt,
        gameId: editing.gameId ? editing.gameId : null,
        title: editing.title,
        body: editing.body,
        pinned: editing.pinned,
      });
      requestAchievementsRefresh();
      setEditing(null);
      pushToast("success", t("notes.saved"));
      await load();
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const togglePin = async (item: GlobalNoteView): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      await readingApi.setGlobalNotePinned(item.note.meta.id, !item.note.pinned);
      await load();
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (item: GlobalNoteView): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      await readingApi.deleteGlobalNote(item.note.meta.id, item.note.meta.updated_at);
      setConfirmDeleteId(null);
      pushToast("success", t("notes.deleted"));
      await load();
    } catch (caught) {
      pushToast("error", caught instanceof IpcError ? caught.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const startEdit = (item: GlobalNoteView): void => {
    setEditing({
      id: item.note.meta.id,
      expectedUpdatedAt: item.note.meta.updated_at,
      gameId: item.note.game_id ?? "",
      title: item.note.title,
      body: item.note.body,
      pinned: item.note.pinned,
    });
  };

  const canSave =
    editing !== null &&
    !busy &&
    (editing.title.trim().length > 0 || editing.body.trim().length > 0);

  return (
    <div className="global-notes" style={{ display: "grid", gap: "var(--space-4)" }}>
      <Card>
        <p className="body" style={{ marginBottom: "var(--space-3)" }}>
          {t("notes.intro")}
        </p>

        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--space-3)",
            alignItems: "center",
          }}
        >
          <div className="search" style={{ flex: "1 1 220px", minWidth: 0 }}>
            <span className="search__icon">
              <Icon name="search" />
            </span>
            <input
              className="input"
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("notes.searchPlaceholder")}
              aria-label={t("notes.searchLabel")}
            />
          </div>

          <select
            className="select"
            style={{ width: "auto", maxWidth: "100%" }}
            value={scope}
            aria-label={t("notes.filterLabel")}
            onChange={(event) => setScope(event.target.value)}
          >
            <option value="all">{t("notes.filterAll")}</option>
            <option value="global">{t("notes.filterGlobal")}</option>
            {cards.map((card) => (
              <option key={card.game.meta.id} value={card.game.meta.id}>
                {card.game.title}
              </option>
            ))}
          </select>

          {editing === null ? (
            <Button
              variant="primary"
              onClick={() => {
                setEditing({
                  ...emptyDraft(),
                  gameId: scope !== "all" && scope !== "global" ? scope : "",
                });
              }}
            >
              <Icon name="plus" size={14} />
              {t("notes.add")}
            </Button>
          ) : null}
        </div>

        {editing !== null ? (
          <form
            className="reading__form"
            style={{ marginTop: "var(--space-4)" }}
            onSubmit={(event) => void submit(event)}
          >
            <div className="reading__fields">
              <label className="field" style={{ gridColumn: "span 2" }}>
                <span className="field__label">{t("notes.titleLabel")}</span>
                <input
                  className="input"
                  value={editing.title}
                  onChange={(event) =>
                    setEditing({ ...editing, title: event.target.value })
                  }
                  placeholder={t("notes.titlePlaceholder")}
                  maxLength={200}
                  autoFocus
                />
              </label>

              <label className="field">
                <span className="field__label">{t("notes.gameLabel")}</span>
                <select
                  className="select"
                  value={editing.gameId}
                  onChange={(event) =>
                    setEditing({ ...editing, gameId: event.target.value })
                  }
                >
                  <option value="">{t("notes.gameGlobal")}</option>
                  {cards.map((card) => (
                    <option key={card.game.meta.id} value={card.game.meta.id}>
                      {card.game.title}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <label className="field">
              <span className="field__label">{t("notes.bodyLabel")}</span>
              <textarea
                className="textarea reading__journal-input"
                rows={5}
                value={editing.body}
                onChange={(event) =>
                  setEditing({ ...editing, body: event.target.value })
                }
                placeholder={t("notes.bodyPlaceholder")}
                maxLength={12000}
              />
            </label>

            <div
              className="reading__form-actions"
              style={{ justifyContent: "space-between" }}
            >
              <Chip
                selected={editing.pinned}
                onClick={() => setEditing({ ...editing, pinned: !editing.pinned })}
                ariaLabel={t("notes.pinToggle")}
              >
                {t("notes.pinToggle")}
              </Chip>

              <div style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setEditing(null)}
                >
                  {t("notes.cancel")}
                </Button>
                <Button type="submit" variant="primary" disabled={!canSave}>
                  {busy ? t("notes.saving") : t("notes.save")}
                </Button>
              </div>
            </div>
          </form>
        ) : null}
      </Card>

      {loadError ? (
        <p className="notice notice--danger" role="alert">
          {loadError}
        </p>
      ) : notes === null ? (
        <div aria-busy="true" aria-label={t("common.loading")}>
          <div className="skeleton" style={{ height: 96, marginBottom: 12 }} />
          <div className="skeleton" style={{ height: 96 }} />
        </div>
      ) : notes.length === 0 ? (
        query.trim() || scope !== "all" ? (
          <EmptyState
            title={t("notes.emptySearchTitle")}
            body={t("notes.emptySearchBody")}
          />
        ) : (
          <EmptyState
            title={t("notes.emptyTitle")}
            body={t("notes.emptyBody")}
            action={
              editing === null ? (
                <Button variant="primary" onClick={() => setEditing(emptyDraft())}>
                  <Icon name="plus" size={14} />
                  {t("notes.add")}
                </Button>
              ) : undefined
            }
          />
        )
      ) : (
        <ul
          className="reading__entries"
          style={{ display: "grid", gap: "var(--space-3)" }}
        >
          {notes.map((item) => {
            const { note, gameTitle } = item;
            const isConfirming = confirmDeleteId === note.meta.id;
            return (
              <li key={note.meta.id} style={{ padding: 0, borderBottom: "none" }}>
                <Card>
                  <div
                    style={{
                      display: "flex",
                      flexWrap: "wrap",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: "var(--space-2)",
                      marginBottom: "var(--space-2)",
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexWrap: "wrap",
                        alignItems: "center",
                        gap: "var(--space-2)",
                        minWidth: 0,
                      }}
                    >
                      <h3 className="h3" style={{ margin: 0 }}>
                        {note.title.trim() || t("notes.untitled")}
                      </h3>
                      {note.pinned ? (
                        <Chip className="chip--accent">{t("notes.pinnedBadge")}</Chip>
                      ) : null}
                      {note.game_id && gameTitle ? (
                        <Link
                          to={`/game/${note.game_id}`}
                          className="chip chip--quiet"
                          style={{ textDecoration: "none" }}
                        >
                          {gameTitle}
                        </Link>
                      ) : (
                        <Chip className="chip--quiet">{t("notes.globalBadge")}</Chip>
                      )}
                    </div>

                    <span className="caption tnum">
                      {formatDate(note.meta.updated_at, locale)}
                    </span>
                  </div>

                  {note.body.trim() ? (
                    <div className="reading__note">{note.body}</div>
                  ) : null}

                  <div className="reading__entry-actions">
                    <Button
                      variant="ghost"
                      disabled={busy}
                      onClick={() => void togglePin(item)}
                    >
                      {note.pinned ? t("notes.unpin") : t("notes.pin")}
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={busy}
                      onClick={() => startEdit(item)}
                    >
                      {t("notes.edit")}
                    </Button>
                    {isConfirming ? (
                      <>
                        <Button
                          variant="danger"
                          disabled={busy}
                          onClick={() => void remove(item)}
                        >
                          {t("notes.deleteConfirm")}
                        </Button>
                        <Button
                          variant="ghost"
                          disabled={busy}
                          onClick={() => setConfirmDeleteId(null)}
                        >
                          {t("notes.cancel")}
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant="ghost"
                        disabled={busy}
                        onClick={() => setConfirmDeleteId(note.meta.id)}
                      >
                        {t("notes.delete")}
                      </Button>
                    )}
                  </div>
                </Card>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
