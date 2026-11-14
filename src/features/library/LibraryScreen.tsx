import { useMemo, useState } from "react";
import { useNavigate } from "react-router";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { UpdateBanner } from "@/features/update/UpdatePanel";
import { api } from "@/lib/ipc/client";
import { filterCards, sortCards, statusOrder, type SortKey } from "@/lib/library";
import type { GameStatus } from "@/lib/ipc/types";
import { AddGameSheet } from "./AddGameSheet";
import { VirtualLibrary } from "./VirtualLibrary";

/** Library view offering title scan (list), cover comparison (grid), and pinned favorites (shelf). */
export function LibraryScreen() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const cards = useAppStore((state) => state.cards);
  const loaded = useAppStore((state) => state.libraryLoaded);
  const error = useAppStore((state) => state.libraryError);
  const setCards = useAppStore((state) => state.setCards);
  const setLibraryError = useAppStore((state) => state.setLibraryError);

  // Persisted in app store because the router unmounts this screen on navigation.
  const { query, statuses, ratedOnly, view, sort } = useAppStore(
    (state) => state.libraryBrowse,
  );
  const updateLibraryBrowse = useAppStore((state) => state.updateLibraryBrowse);
  const [addOpen, setAddOpen] = useState(false);
  // Reset scrolling for browse changes, not status-array reference churn.
  const collectionKey = `${view}|${query}|${statuses.join(",")}|${ratedOnly ? 1 : 0}|${sort}`;

  // Reuse `lib/library` normalization so card ordering matches tested sort/filter behavior.
  const filtered = useMemo(
    () =>
      // Title reads A→Z; the numeric keys read highest-first.
      sortCards(
        filterCards(cards, { query, statuses, ratedOnly }),
        sort,
        sort !== "title",
      ),
    [cards, query, statuses, ratedOnly, sort],
  );

  const toggleStatus = (status: GameStatus): void => {
    updateLibraryBrowse({
      statuses: statuses.includes(status)
        ? statuses.filter((entry) => entry !== status)
        : [...statuses, status],
    });
  };

  const clearFilters = (): void =>
    updateLibraryBrowse({ query: "", statuses: [], ratedOnly: false });

  const reload = async (): Promise<void> => {
    try {
      setCards(await api.listGames());
    } catch (caught) {
      setLibraryError(caught instanceof Error ? caught.message : t("common.error"));
    }
  };

  if (!loaded) {
    return (
      <div aria-busy="true" aria-label={t("library.loading")}>
        <div className="grid">
          {Array.from({ length: 8 }, (_, index) => (
            <div key={index} style={{ display: "grid", gap: 8 }}>
              <div className="skeleton" style={{ aspectRatio: "2 / 3" }} />
              <div className="skeleton" style={{ height: 12, width: "70%" }} />
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <EmptyState
        title={t("library.errorTitle")}
        body={error}
        action={
          <Button variant="secondary" onClick={() => void reload()}>
            {t("library.retry")}
          </Button>
        }
      />
    );
  }

  if (cards.length === 0) {
    return (
      <>
        <EmptyState
          title={t("library.emptyTitle")}
          body={t("library.emptyBody")}
          action={
            <Button variant="primary" onClick={() => setAddOpen(true)}>
              <Icon name="plus" />
              {t("library.emptyCta")}
            </Button>
          }
        />
        <AddGameSheet open={addOpen} onClose={() => setAddOpen(false)} />
      </>
    );
  }

  const filtersActive = query.trim() !== "" || statuses.length > 0 || ratedOnly;

  return (
    <>
      <UpdateBanner />

      <div className="library__bar">
        <div className="search">
          <span className="search__icon">
            <Icon name="search" />
          </span>
          <input
            className="input"
            type="search"
            value={query}
            onChange={(event) => updateLibraryBrowse({ query: event.target.value })}
            placeholder={t("library.search")}
            aria-label={t("library.searchLabel")}
          />
        </div>

        <Button
          variant="ghost"
          onClick={() => navigate("/big-picture")}
          title={t("bp.entryHint")}
          aria-label={t("bp.title")}
        >
          <Icon name="screen" />
          <span className="bp-entry__label">{t("bp.entry")}</span>
        </Button>

        <div className="tabs" role="tablist" aria-label={t("library.view")}>
          {(
            [
              ["grid", "library.viewGrid", "grid"],
              ["list", "library.viewList", "list"],
              ["shelf", "library.viewShelf", "shelf"],
            ] as const
          ).map(([mode, labelKey, icon]) => (
            <button
              key={mode}
              type="button"
              role="tab"
              aria-selected={view === mode}
              className="tabs__tab"
              onClick={() => updateLibraryBrowse({ view: mode })}
            >
              <Icon name={icon} size={14} />
              {t(labelKey)}
            </button>
          ))}
        </div>

        <label className="field" style={{ gap: 4 }}>
          <span className="visually-hidden">{t("library.sort")}</span>
          <select
            className="select"
            value={sort}
            onChange={(event) =>
              updateLibraryBrowse({ sort: event.target.value as SortKey })
            }
            aria-label={t("library.sort")}
          >
            <option value="recent">{t("library.sortRecent")}</option>
            <option value="title">{t("library.sortTitle")}</option>
            <option value="rating">{t("library.sortRating")}</option>
            <option value="playtime">{t("library.sortPlaytime")}</option>
            <option value="started">{t("library.sortStarted")}</option>
          </select>
        </label>

        <Button variant="primary" onClick={() => setAddOpen(true)}>
          <Icon name="plus" />
          {t("add.title")}
        </Button>
      </div>

      <div className="library__filters">
        {statusOrder.map((status) => (
          <Chip
            key={status}
            selected={statuses.includes(status)}
            onClick={() => toggleStatus(status)}
          >
            {t(`status.${status}`)}
          </Chip>
        ))}
        <Chip
          selected={ratedOnly}
          onClick={() => updateLibraryBrowse({ ratedOnly: !ratedOnly })}
        >
          {t("library.ratedOnly")}
        </Chip>
        <span className="library__count tnum">
          {t("library.count", { count: filtered.length })}
        </span>
      </div>

      {filtered.length === 0 ? (
        <EmptyState
          title={t("library.noMatchTitle")}
          body={t("library.noMatchBody")}
          action={
            <Button variant="secondary" onClick={clearFilters}>
              {t("library.noMatchCta")}
            </Button>
          }
        />
      ) : (
        <VirtualLibrary
          key={view}
          cards={filtered}
          view={view}
          resetKey={collectionKey}
        />
      )}

      {filtersActive ? (
        <p className="library__count" style={{ marginTop: "var(--space-4)" }}>
          <button type="button" className="btn btn--ghost" onClick={clearFilters}>
            {t("library.noMatchCta")}
          </button>
        </p>
      ) : null}

      <AddGameSheet open={addOpen} onClose={() => setAddOpen(false)} />
    </>
  );
}
