import { useEffect, useRef, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Sheet } from "@/components/Sheet";
import { Chip } from "@/components/Chip";
import { Icon } from "@/components/Icon";
import { api, IpcError } from "@/lib/ipc/client";
import { isExplicitHit, searchHits } from "@/lib/vndbSearch";
import type { GameStatus, SearchHit } from "@/lib/ipc/types";
import { statusOrder } from "@/lib/library";

interface AddGameSheetProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Add a game: search VNDB, or type a title by hand.
 *
 * Search is explicit (button or Enter), not as-you-type: the API allows roughly
 * 200 requests per 5 minutes, and firing a request per keystroke would waste
 * that budget and get the app throttled.
 */
export function AddGameSheet({ open, onClose }: AddGameSheetProps) {
  const { t } = useI18n();
  const nsfw = useAppStore((state) => state.prefs.nsfw);
  const pushToast = useAppStore((state) => state.pushToast);
  const setCards = useAppStore((state) => state.setCards);

  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [page, setPage] = useState(1);
  const [more, setMore] = useState(false);
  const [fromCache, setFromCache] = useState(false);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState<string | null>(null);

  // Manual entry stays available when VNDB is unreachable.
  const [manualTitle, setManualTitle] = useState("");
  const [manualStatus, setManualStatus] = useState<GameStatus>("wishlist");
  const [manualError, setManualError] = useState<string | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const refreshLibrary = async (): Promise<void> => {
    try {
      setCards(await api.listGames());
    } catch {
      // The library reload is a convenience; the sheet should not fail on it.
    }
  };

  const runSearch = async (nextPage = 1): Promise<void> => {
    const trimmed = query.trim();
    if (trimmed.length === 0) return;
    setSearching(true);
    setError(null);
    try {
      const response = await searchHits(trimmed, nextPage);
      setHits((current) =>
        nextPage === 1 ? response.hits : [...current, ...response.hits],
      );
      setMore(response.more);
      setFromCache(response.from_cache);
      setPage(nextPage);
      setSearched(true);
    } catch (caught) {
      setError(
        caught instanceof IpcError ? caught.message : t("add.searchFailed"),
      );
      setSearched(true);
    } finally {
      setSearching(false);
    }
  };

  const importHit = async (hit: SearchHit): Promise<void> => {
    setImporting(hit.vn_id);
    try {
      const outcome = await api.vndbImport(hit.vn_id);
      await refreshLibrary();
      pushToast(
        "success",
        t(outcome.already_owned ? "add.importedOwned" : "add.imported", {
          title: hit.title,
        }),
      );
      if (outcome.cover_error) {
        pushToast("error", t("add.coverFailed"));
      }
      onClose();
    } catch (caught) {
      pushToast(
        "error",
        caught instanceof IpcError ? caught.message : t("add.failed"),
      );
    } finally {
      setImporting(null);
    }
  };

  const addManual = async (event: React.FormEvent): Promise<void> => {
    event.preventDefault();
    if (manualTitle.trim().length === 0) {
      setManualError(t("add.nameRequired"));
      return;
    }
    setManualError(null);
    try {
      await api.createGame({ title: manualTitle.trim(), status: manualStatus });
      await refreshLibrary();
      setManualTitle("");
      setManualStatus("wishlist");
      onClose();
    } catch (caught) {
      const message = caught instanceof IpcError ? caught.message : t("add.failed");
      setManualError(message);
      pushToast("error", message);
    }
  };

  return (
    <Sheet open={open} title={t("add.title")} onClose={onClose}>
      <form
        className="field"
        onSubmit={(event) => {
          event.preventDefault();
          void runSearch(1);
        }}
      >
        <label className="field__label" htmlFor="vndb-query">
          {t("add.searchLabel")}
        </label>
        <div style={{ display: "flex", gap: 8 }}>
          <div className="search" style={{ flex: 1 }}>
            <span className="search__icon">
              <Icon name="search" />
            </span>
            <input
              id="vndb-query"
              ref={inputRef}
              className="input"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("add.searchPlaceholder")}
              autoComplete="off"
            />
          </div>
          <Button variant="primary" type="submit" disabled={searching}>
            {searching ? t("add.searching") : t("add.searchLabel")}
          </Button>
        </div>
        <span className="settings__hint">{t("add.searchHint")}</span>
      </form>

      {fromCache ? (
        <p className="caption" style={{ color: "var(--warning)" }}>
          {t("add.offlineResults")}
        </p>
      ) : null}

      {error ? (
        <p style={{ color: "var(--danger)", fontSize: 12 }}>{error}</p>
      ) : null}

      {searching && hits.length === 0 ? (
        <div aria-busy="true" aria-label={t("add.searching")}>
          {Array.from({ length: 3 }, (_, index) => (
            <div
              key={index}
              style={{ display: "flex", gap: 12, marginBottom: 12 }}
            >
              <div
                className="skeleton"
                style={{ width: 48, aspectRatio: "2 / 3" }}
              />
              <div style={{ flex: 1, display: "grid", gap: 8 }}>
                <div className="skeleton" style={{ height: 12, width: "60%" }} />
                <div className="skeleton" style={{ height: 10, width: "40%" }} />
              </div>
            </div>
          ))}
        </div>
      ) : null}

      {searched && !searching && hits.length === 0 && !error ? (
        <p className="body">{t("add.noResults")}</p>
      ) : null}

      {hits.length > 0 ? (
        <ul style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {hits.map((hit) => {
            const hidden = isExplicitHit(hit, nsfw);
            return (
              <li
                key={hit.vn_id}
                style={{
                  display: "grid",
                  gridTemplateColumns: "48px 1fr auto",
                  gap: 12,
                  alignItems: "center",
                  padding: 8,
                  border: "1px solid var(--border)",
                  borderRadius: "var(--radius-m)",
                }}
              >
                <div
                  style={{
                    width: 48,
                    height: 72,
                    borderRadius: "var(--radius-xs)",
                    overflow: "hidden",
                    background: "var(--surface-2)",
                    display: "grid",
                    placeItems: "center",
                  }}
                >
                  {hit.image_url && !hidden ? (
                    <img
                      src={hit.image_url}
                      alt=""
                      width={48}
                      height={72}
                      loading="lazy"
                      style={{ width: "100%", height: "100%", objectFit: "cover" }}
                    />
                  ) : (
                    <Icon name={hidden ? "eyeOff" : "library"} size={16} />
                  )}
                </div>

                <div style={{ minWidth: 0 }}>
                  <div className="row__title" title={hit.title}>
                    {hit.title}
                  </div>
                  {hit.alttitle ? (
                    <div className="row__sub">{hit.alttitle}</div>
                  ) : null}
                  <div className="cover__meta">
                    {hit.released ?? t("common.unknown")}
                    {hit.length_minutes
                      ? ` · ${t("add.length", {
                          hours: Math.round((hit.length_minutes / 60) * 10) / 10,
                        })}`
                      : ""}
                  </div>
                </div>

                {hit.in_library ? (
                  <Chip>{t("add.inLibrary")}</Chip>
                ) : (
                  <Button
                    variant="secondary"
                    onClick={() => void importHit(hit)}
                    disabled={importing !== null}
                  >
                    {importing === hit.vn_id ? t("add.importing") : t("add.import")}
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      ) : null}

      {more ? (
        <Button
          variant="ghost"
          onClick={() => void runSearch(page + 1)}
          disabled={searching}
        >
          {t("add.more")}
        </Button>
      ) : null}

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          margin: "8px 0",
          color: "var(--text-muted)",
          fontSize: "var(--text-xs)",
        }}
      >
        <span style={{ flex: 1, height: 1, background: "var(--border)" }} />
        {t("add.manualDivider")}
        <span style={{ flex: 1, height: 1, background: "var(--border)" }} />
      </div>

      <form className="field" onSubmit={(event) => void addManual(event)}>
        <label className="field__label" htmlFor="add-title">
          {t("add.nameLabel")}
        </label>
        <input
          id="add-title"
          className="input"
          value={manualTitle}
          onChange={(event) => setManualTitle(event.target.value)}
          placeholder={t("add.namePlaceholder")}
          autoComplete="off"
          aria-invalid={manualError ? "true" : undefined}
        />
        <span className="settings__hint">{t("add.manualHint")}</span>
        {manualError ? (
          <p style={{ color: "var(--danger)", fontSize: 12 }}>{manualError}</p>
        ) : null}

        <span className="field__label">{t("add.statusLabel")}</span>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          {statusOrder.map((candidate) => (
            <Chip
              key={candidate}
              selected={manualStatus === candidate}
              onClick={() => setManualStatus(candidate)}
            >
              {t(`status.${candidate}`)}
            </Chip>
          ))}
        </div>

        <Button variant="primary" type="submit">
          {t("add.submit")}
        </Button>
      </form>
    </Sheet>
  );
}
