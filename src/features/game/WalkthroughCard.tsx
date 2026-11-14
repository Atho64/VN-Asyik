import { useEffect, useState } from "react";
import { useI18n } from "@/app/useI18n";
import { useAppStore } from "@/app/store";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/ipc/client";
import type {
  WalkthroughView,
  WalkthroughIndexEntry,
  WalkthroughRoute,
  WalkthroughItem,
} from "@/lib/ipc/types";

interface WalkthroughCardProps {
  gameId: string;
  vnId?: string | null;
}

type LangMode = "original" | "ai" | "google";

function formatUrlSlug(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname.replace(/^\/game\//, "").replace(/^\//, "");
    return path || url;
  } catch {
    return url;
  }
}

function getPlatformBadge(url: string): { label: string; isConsole: boolean } {
  const lower = url.toLowerCase();
  if (lower.includes("/switch/")) return { label: "Switch", isConsole: true };
  if (lower.includes("/ps4/")) return { label: "PS4", isConsole: true };
  if (lower.includes("/ps3/")) return { label: "PS3", isConsole: true };
  if (lower.includes("/vita/")) return { label: "PS Vita", isConsole: true };
  if (lower.includes("/psp/")) return { label: "PSP", isConsole: true };
  return { label: "PC", isConsole: false };
}

export function WalkthroughCard({ gameId, vnId }: WalkthroughCardProps) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [data, setData] = useState<WalkthroughView | null>(null);
  const [loading, setLoading] = useState(true);
  const [fetching, setFetching] = useState(false);
  const [translating, setTranslating] = useState(false);
  const [candidates, setCandidates] = useState<WalkthroughIndexEntry[]>([]);
  const [searchingCandidate, setSearchingCandidate] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<WalkthroughIndexEntry[] | null>(null);
  const [isSearching, setIsSearching] = useState(false);
  const [showSwitchSearch, setShowSwitchSearch] = useState(false);
  const [selectedRouteId, setSelectedRouteId] = useState<string>("all");
  const [langMode, setLangMode] = useState<LangMode>("original");
  const [customUrl, setCustomUrl] = useState("");
  const [showInput, setShowInput] = useState(false);

  const targetLang = locale === "id" ? "id" : "en";

  useEffect(() => {
    let cancelled = false;

    api
      .walkthroughGet(gameId, vnId)
      .then((res: WalkthroughView | null) => {
        if (cancelled) return;
        setData(res);
        setLoading(false);

        if (res) {
          const aiKey = `ai_${targetLang}`;
          const gKey = `google_${targetLang}`;
          if (res.translations[aiKey]) {
            setLangMode("ai");
          } else if (res.translations[gKey]) {
            setLangMode("google");
          }
        } else {
          setSearchingCandidate(true);
          void api
            .walkthroughSearchCandidate(gameId, vnId)
            .then((matches: WalkthroughIndexEntry[] | WalkthroughIndexEntry | null) => {
              if (cancelled) return;
              if (Array.isArray(matches)) {
                setCandidates(matches);
              } else if (matches) {
                setCandidates([matches]);
              } else {
                setCandidates([]);
              }
            })
            .finally(() => {
              if (!cancelled) setSearchingCandidate(false);
            });
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setLoading(false);
          pushToast("error", String(err));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [gameId, vnId, targetLang, pushToast, t]);

  const handleFetch = async (urlToFetch?: string | null) => {
    setFetching(true);
    try {
      const res = await api.walkthroughFetch(gameId, vnId, urlToFetch);
      setData(res);
      setCandidates([]);
      setSearchResults(null);
      setSearchQuery("");
      setShowInput(false);
      setShowSwitchSearch(false);
      pushToast("success", t("walkthrough.fetch"));
    } catch (err: unknown) {
      pushToast("error", String(err));
    } finally {
      setFetching(false);
    }
  };

  const handleSearch = async (query: string) => {
    const trimmed = query.trim();
    if (!trimmed) {
      setSearchResults(null);
      return;
    }
    setIsSearching(true);
    try {
      const results = await api.walkthroughSearch(trimmed);
      setSearchResults(results);
    } catch (err: unknown) {
      pushToast("error", String(err));
    } finally {
      setIsSearching(false);
    }
  };

  const handleTranslate = async (provider: "ai" | "google") => {
    setTranslating(true);
    try {
      const res = await api.walkthroughTranslate(gameId, provider, targetLang, vnId);
      setData(res);
      setLangMode(provider);
      pushToast(
        "success",
        provider === "ai" ? t("walkthrough.translateAi") : t("walkthrough.translateGoogle"),
      );
    } catch (err: unknown) {
      pushToast("error", String(err));
    } finally {
      setTranslating(false);
    }
  };

  const handleToggle = async (itemId: string) => {
    if (!data) return;

    const prevChecked = data.checkedItems;
    const isChecked = prevChecked.includes(itemId);
    const nextChecked = isChecked
      ? prevChecked.filter((id) => id !== itemId)
      : [...prevChecked, itemId];

    setData({ ...data, checkedItems: nextChecked });

    try {
      const synced = await api.walkthroughToggleItem(gameId, itemId);
      setData((curr) => (curr ? { ...curr, checkedItems: synced } : null));
    } catch (err: unknown) {
      setData((curr) => (curr ? { ...curr, checkedItems: prevChecked } : null));
      pushToast("error", String(err));
    }
  };

  const handleClearProgress = async () => {
    if (!data || !window.confirm(t("walkthrough.resetConfirm"))) return;
    try {
      await api.walkthroughClearProgress(gameId);
      setData({ ...data, checkedItems: [] });
    } catch (err: unknown) {
      pushToast("error", String(err));
    }
  };

  const currentRoutes: WalkthroughRoute[] = (() => {
    if (!data) return [];
    if (langMode === "ai") {
      const aiKey = `ai_${targetLang}`;
      if (data.translations[aiKey]) return data.translations[aiKey];
    } else if (langMode === "google") {
      const gKey = `google_${targetLang}`;
      if (data.translations[gKey]) return data.translations[gKey];
    }
    return data.routes;
  })();

  const glossaryCount = data ? Object.keys(data.characterGlossary).length : 0;

  const displayedRoutes =
    selectedRouteId === "all"
      ? currentRoutes
      : currentRoutes.filter((r) => r.id === selectedRouteId);

  const allChoiceIds: string[] = [];
  for (const r of currentRoutes) {
    for (const item of r.items) {
      if (item.kind === "choice") {
        allChoiceIds.push(item.id);
      }
    }
  }
  const totalChoices = allChoiceIds.length;
  const completedChoices = data
    ? data.checkedItems.filter((id) => allChoiceIds.includes(id)).length
    : 0;

  const renderSearchSection = (inSwitchMode = false) => (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-3)",
        ...(inSwitchMode
          ? {
              padding: "var(--space-3)",
              background: "var(--color-surface-raised)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              marginBottom: "var(--space-3)",
            }
          : {}),
      }}
    >
      {inSwitchMode && (
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            flexWrap: "wrap",
            gap: "var(--space-2)",
          }}
        >
          <span style={{ fontSize: "var(--font-size-xs)", fontWeight: 600 }}>
            {t("walkthrough.searchOtherGuide")}
          </span>
          <Button
            size="icon"
            variant="ghost"
            onClick={() => setShowSwitchSearch(false)}
            aria-label={t("common.close")}
          >
            <Icon name="close" size={12} />
          </Button>
        </div>
      )}

      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleSearch(searchQuery);
        }}
        style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}
      >
        <div style={{ position: "relative", flex: 1, minWidth: "8rem" }}>
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("walkthrough.searchPlaceholder")}
            aria-label={t("walkthrough.searchLabel")}
            style={{
              width: "100%",
              padding: "var(--space-2) var(--space-3) var(--space-2) 32px",
              background: inSwitchMode ? "var(--color-surface)" : "var(--color-surface-raised)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              color: "var(--color-text)",
              fontSize: "var(--font-size-sm)",
            }}
          />
          <div
            style={{
              position: "absolute",
              left: "10px",
              top: "50%",
              transform: "translateY(-50%)",
              color: "var(--color-text-dim)",
              pointerEvents: "none",
              display: "flex",
              alignItems: "center",
            }}
          >
            <Icon name="search" size={14} />
          </div>
        </div>
        <Button
          type="submit"
          variant="secondary"
          disabled={isSearching || !searchQuery.trim()}
        >
          {isSearching ? t("walkthrough.searching") : t("walkthrough.searchBtn")}
        </Button>
      </form>

      {searchResults !== null ? (
        searchResults.length === 0 ? (
          <div
            style={{
              padding: "var(--space-3)",
              textAlign: "center",
              color: "var(--color-text-dim)",
              fontSize: "var(--font-size-xs)",
              background: inSwitchMode ? "var(--color-surface)" : "var(--color-surface-raised)",
              border: "1px dashed var(--color-border)",
              borderRadius: "var(--radius-md)",
            }}
          >
            {t("walkthrough.noResults")}
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
            <span style={{ fontSize: "var(--font-size-xs)", fontWeight: 600, color: "var(--color-text-dim)" }}>
              {t("walkthrough.searchResults", { count: String(searchResults.length) })}
            </span>
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)", maxHeight: "280px", overflowY: "auto" }}>
              {searchResults.map((entry) => (
                <div
                  key={entry.url}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    flexWrap: "wrap",
                    padding: "var(--space-2) var(--space-3)",
                    background: inSwitchMode ? "var(--color-surface)" : "var(--color-surface-raised)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    gap: "var(--space-2)",
                  }}
                >
                  <div style={{ minWidth: 0, flex: 1 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", minWidth: 0 }}>
                      <span
                        style={{
                          fontSize: "10px",
                          fontWeight: 600,
                          padding: "1px 6px",
                          borderRadius: "var(--radius-sm)",
                          background: getPlatformBadge(entry.url).isConsole ? "var(--accent-soft)" : "var(--success-soft)",
                          color: getPlatformBadge(entry.url).isConsole ? "var(--accent)" : "var(--success)",
                          letterSpacing: "0.03em",
                          flexShrink: 0,
                        }}
                      >
                        {getPlatformBadge(entry.url).label}
                      </span>
                      <span style={{ fontWeight: 500, fontSize: "var(--font-size-sm)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                        {entry.title}
                      </span>
                    </div>
                    <div
                    style={{
                      fontSize: "var(--font-size-xs)",
                      color: "var(--color-text-dim)",
                      fontFamily: "monospace",
                      paddingLeft: "var(--space-1)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                      {formatUrlSlug(entry.url)}
                    </div>
                  </div>
                  <Button
                                        variant="primary"
                    onClick={() => void handleFetch(entry.url)}
                    disabled={fetching}
                  >
                    {fetching ? t("walkthrough.fetching") : t("walkthrough.select")}
                  </Button>
                </div>
              ))}
            </div>
          </div>
        )
      ) : null}

      {!searchResults && candidates.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <span style={{ fontSize: "var(--font-size-xs)", fontWeight: 600, color: "var(--color-accent)" }}>
              {t("walkthrough.candidatesFound", { count: String(candidates.length) })}
            </span>
            <span
              style={{
                fontSize: "10px",
                color: "var(--color-accent)",
                fontWeight: 600,
                letterSpacing: "0.02em",
              }}
            >
              {t("walkthrough.matchSource")}
            </span>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)", maxHeight: "240px", overflowY: "auto" }}>
            {candidates.map((cand) => (
              <div
                key={cand.url}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  flexWrap: "wrap",
                  padding: "var(--space-2) var(--space-3)",
                  background: inSwitchMode ? "var(--color-surface)" : "var(--color-surface-raised)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "var(--radius-md)",
                  gap: "var(--space-2)",
                }}
              >
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", minWidth: 0 }}>
                    <span
                      style={{
                        fontSize: "10px",
                        fontWeight: 600,
                        padding: "1px 6px",
                        borderRadius: "var(--radius-sm)",
                        background: getPlatformBadge(cand.url).isConsole ? "var(--accent-soft)" : "var(--success-soft)",
                        color: getPlatformBadge(cand.url).isConsole ? "var(--accent)" : "var(--success)",
                        letterSpacing: "0.03em",
                        flexShrink: 0,
                      }}
                    >
                      {getPlatformBadge(cand.url).label}
                    </span>
                    <span style={{ fontWeight: 500, fontSize: "var(--font-size-sm)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                      {cand.title}
                    </span>
                  </div>
                  <div
                    style={{
                      fontSize: "var(--font-size-xs)",
                      color: "var(--color-text-dim)",
                      fontFamily: "monospace",
                      paddingLeft: "var(--space-1)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {formatUrlSlug(cand.url)}
                  </div>
                </div>
                <Button
                                    variant="primary"
                  onClick={() => void handleFetch(cand.url)}
                  disabled={fetching}
                >
                  {fetching ? t("walkthrough.fetching") : t("walkthrough.fetch")}
                </Button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div style={{ borderTop: "1px solid var(--color-border)", paddingTop: "var(--space-2)" }}>
        {!showInput ? (
          <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                flexWrap: "wrap",
                gap: "var(--space-2)",
              }}
            >
            <Button
              variant="ghost"
                            onClick={() => setShowInput(true)}
              style={{ color: "var(--color-text-dim)", fontSize: "var(--font-size-xs)" }}
            >
              <Icon name="plus" size={12} />
              {t("walkthrough.orManualUrl")}
            </Button>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--color-text-dim)" }}>
              {t("walkthrough.autoSearchTip")}
            </span>
          </div>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const trimmed = customUrl.trim();
              if (!trimmed) {
                pushToast("error", t("walkthrough.urlRequired"));
                return;
              }
              void handleFetch(trimmed);
            }}
            style={{ display: "flex", gap: "var(--space-2)", flexWrap: "wrap" }}
          >
            <input
              type="url"
              value={customUrl}
              onChange={(e) => setCustomUrl(e.target.value)}
              placeholder={t("walkthrough.customUrlPlaceholder")}
              aria-label={t("walkthrough.customUrlLabel")}
              style={{
                flex: 1,
                minWidth: "10rem",
                padding: "var(--space-2) var(--space-3)",
                background: inSwitchMode ? "var(--color-surface)" : "var(--color-surface-raised)",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-md)",
                color: "var(--color-text)",
                fontSize: "var(--font-size-sm)",
              }}
              required
            />
            <Button type="submit" variant="primary" disabled={fetching || !customUrl.trim()}>
              {fetching ? t("walkthrough.fetching") : t("walkthrough.fetch")}
            </Button>
            <Button type="button" variant="ghost" onClick={() => setShowInput(false)}>
              {t("common.cancel")}
            </Button>
          </form>
        )}
      </div>
    </div>
  );

  return (
    <Card>
      <CardHeader
        title={t("walkthrough.title")}
        aside={
          data ? (
            <div
              style={{
                display: "flex",
                gap: "var(--space-2)",
                alignItems: "center",
                flexWrap: "wrap",
              }}
            >
              <span
                style={{
                  fontSize: "var(--font-size-xs)",
                  color: "var(--color-text-dim)",
                  fontWeight: 500,
                }}
              >
                {t("walkthrough.progress", {
                  checked: String(completedChoices),
                  total: String(totalChoices),
                })}
              </span>
              <Button
                size="icon"
                variant={showSwitchSearch ? "secondary" : "ghost"}
                onClick={() => setShowSwitchSearch((prev) => !prev)}
                title={t("walkthrough.switchGuide")}
                aria-label={t("walkthrough.switchGuide")}
              >
                <Icon name="search" size={14} />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                onClick={() => void handleFetch(data.sourceUrl)}
                disabled={fetching}
                title={t("walkthrough.reload")}
                aria-label={t("walkthrough.reload")}
              >
                <Icon name="spark" size={14} />
              </Button>
            </div>
          ) : null
        }
      />

      <p style={{ color: "var(--color-text-dim)", margin: "0 0 var(--space-3) 0", fontSize: "var(--font-size-xs)", lineHeight: 1.5 }}>
        {t("walkthrough.intro")}
      </p>

      {loading ? (
        <div style={{ padding: "var(--space-4)", textAlign: "center", color: "var(--color-text-dim)" }}>
          {t("walkthrough.fetching")}
        </div>
      ) : !data ? (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <p style={{ color: "var(--color-text-dim)", margin: 0, fontSize: "var(--font-size-sm)" }}>
            {t("walkthrough.empty")}
          </p>

          {searchingCandidate && (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
                fontSize: "var(--font-size-xs)",
                color: "var(--color-text-dim)",
              }}
            >
              <Icon name="search" size={13} />
              <span>{t("walkthrough.searchingDirectory")}</span>
            </div>
          )}

          {renderSearchSection(false)}
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
          {showSwitchSearch && renderSearchSection(true)}

          <div
            style={{
              display: "flex",
              flexWrap: "wrap",
              gap: "var(--space-2)",
              alignItems: "center",
              justifyContent: "space-between",
              paddingBottom: "var(--space-2)",
              borderBottom: "1px solid var(--color-border)",
            }}
          >
            <div
              style={{
                display: "inline-flex",
                flexWrap: "wrap",
                gap: "2px",
                borderRadius: "var(--radius-md)",
                background: "var(--color-surface-raised)",
                padding: "2px",
              }}
            >
              <button
                type="button"
                onClick={() => setLangMode("original")}
                style={{
                  minHeight: "var(--touch-target)",
                  padding: "var(--space-1) var(--space-3)",
                  borderRadius: "var(--radius-sm)",
                  border: "none",
                  fontSize: "var(--font-size-xs)",
                  cursor: "pointer",
                  fontWeight: langMode === "original" ? 600 : 400,
                  background: langMode === "original" ? "var(--color-surface)" : "transparent",
                  color: langMode === "original" ? "var(--color-text)" : "var(--color-text-dim)",
                }}
              >
                {t("walkthrough.langOriginal")}
              </button>
              <button
                type="button"
                onClick={() => {
                  const key = `ai_${targetLang}`;
                  if (data.translations[key]) {
                    setLangMode("ai");
                  } else {
                    void handleTranslate("ai");
                  }
                }}
                disabled={translating}
                style={{
                  minHeight: "var(--touch-target)",
                  padding: "var(--space-1) var(--space-3)",
                  borderRadius: "var(--radius-sm)",
                  border: "none",
                  fontSize: "var(--font-size-xs)",
                  cursor: translating ? "not-allowed" : "pointer",
                  fontWeight: langMode === "ai" ? 600 : 400,
                  background: langMode === "ai" ? "var(--color-surface)" : "transparent",
                  color: langMode === "ai" ? "var(--color-accent)" : "var(--color-text-dim)",
                }}
              >
                {t("walkthrough.langAi")}
              </button>
              <button
                type="button"
                onClick={() => {
                  const key = `google_${targetLang}`;
                  if (data.translations[key]) {
                    setLangMode("google");
                  } else {
                    void handleTranslate("google");
                  }
                }}
                disabled={translating}
                style={{
                  minHeight: "var(--touch-target)",
                  padding: "var(--space-1) var(--space-3)",
                  borderRadius: "var(--radius-sm)",
                  border: "none",
                  fontSize: "var(--font-size-xs)",
                  cursor: translating ? "not-allowed" : "pointer",
                  fontWeight: langMode === "google" ? 600 : 400,
                  background: langMode === "google" ? "var(--color-surface)" : "transparent",
                  color: langMode === "google" ? "var(--color-text)" : "var(--color-text-dim)",
                }}
              >
                {t("walkthrough.langGoogle")}
              </button>
            </div>

            <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
              {glossaryCount > 0 ? (
                <span
                  style={{
                    fontSize: "var(--font-size-xs)",
                    color: "var(--color-text-dim)",
                    padding: "2px var(--space-2)",
                    borderRadius: "var(--radius-sm)",
                    background: "var(--color-surface-raised)",
                  }}
                  title={Object.entries(data.characterGlossary)
                    .map(([k, v]) => `${k} -> ${v}`)
                    .join(", ")}
                >
                  {t("walkthrough.glossaryActive", { count: String(glossaryCount) })}
                </span>
              ) : null}

              {translating ? (
                <span style={{ fontSize: "var(--font-size-xs)", color: "var(--color-accent)" }}>
                  {t("walkthrough.translating")}
                </span>
              ) : null}
            </div>
          </div>

          {currentRoutes.length > 1 ? (
            <div
              style={{
                display: "flex",
                gap: "var(--space-1)",
                overflowX: "auto",
                paddingBottom: "var(--space-1)",
              }}
            >
              <Button
                variant={selectedRouteId === "all" ? "primary" : "ghost"}
                onClick={() => setSelectedRouteId("all")}
              >
                {t("walkthrough.allRoutes")}
              </Button>
              {currentRoutes.map((r) => (
                <Button
                  key={r.id}
                  variant={selectedRouteId === r.id ? "primary" : "ghost"}
                  onClick={() => setSelectedRouteId(r.id)}
                >
                  {r.title}
                </Button>
              ))}
            </div>
          ) : null}

          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
            {displayedRoutes.map((route) => (
              <div
                key={route.id}
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--space-2)",
                  background: "var(--color-surface-raised)",
                  padding: "var(--space-3)",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-border)",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    flexWrap: "wrap",
                    gap: "var(--space-2)",
                    borderBottom: "1px solid var(--color-border)",
                    paddingBottom: "var(--space-2)",
                  }}
                >
                  <span style={{ fontWeight: 600, fontSize: "var(--font-size-sm)" }}>
                    {route.title}
                  </span>
                  <span style={{ fontSize: "var(--font-size-xs)", color: "var(--color-text-dim)" }}>
                    {route.items.filter((i) => i.kind === "choice").length} {t("walkthrough.kind.choice")}
                  </span>
                </div>

                <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
                  {route.items.map((item) => (
                    <WalkthroughItemRow
                      key={item.id}
                      item={item}
                      isChecked={data.checkedItems.includes(item.id)}
                      onToggle={() => void handleToggle(item.id)}
                      t={t}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>

          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              flexWrap: "wrap",
              gap: "var(--space-2)",
              paddingTop: "var(--space-2)",
              borderTop: "1px solid var(--color-border)",
              fontSize: "var(--font-size-xs)",
            }}
          >
            <a
              href={data.sourceUrl}
              target="_blank"
              rel="noopener noreferrer"
              onClick={(e) => {
                e.preventDefault();
                void api.openExternal(data.sourceUrl);
              }}
              style={{ color: "var(--color-text-dim)", textDecoration: "none" }}
            >
              {t("walkthrough.source", { title: data.title })}
            </a>

            {completedChoices > 0 ? (
              <Button variant="ghost" onClick={() => void handleClearProgress()}>
                {t("walkthrough.resetProgress")}
              </Button>
            ) : null}
          </div>
        </div>
      )}
    </Card>
  );
}

function WalkthroughItemRow({
  item,
  isChecked,
  onToggle,
  t,
}: {
  item: WalkthroughItem;
  isChecked: boolean;
  onToggle: () => void;
  t: (k: string) => string;
}) {
  if (item.kind === "choice") {
    return (
      <label
        style={{
          display: "flex",
          alignItems: "center",
          minHeight: "var(--touch-target)",
          gap: "var(--space-2)",
          padding: "var(--space-2)",
          borderRadius: "var(--radius-sm)",
          cursor: "pointer",
          background: isChecked ? "var(--color-surface)" : "transparent",
          opacity: isChecked ? 0.65 : 1,
          transition: "opacity 120ms ease, background 120ms ease",
        }}
      >
        <input
          type="checkbox"
          checked={isChecked}
          onChange={onToggle}
          style={{
            marginTop: "3px",
            accentColor: "var(--color-accent)",
            cursor: "pointer",
          }}
        />
        <span
          style={{
            fontSize: "var(--font-size-sm)",
            lineHeight: "1.4",
            color: "var(--color-text)",
            textDecoration: isChecked ? "line-through" : "none",
          }}
        >
          {item.text}
        </span>
      </label>
    );
  }

  if (item.kind === "save") {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          margin: "var(--space-1) 0",
          padding: "var(--space-1) var(--space-2)",
        }}
      >
        <span
          style={{
            display: "inline-block",
            fontSize: "var(--font-size-xs)",
            fontWeight: 700,
            color: "var(--danger)",
            background: "var(--danger-soft)",
            padding: "2px 6px",
            borderRadius: "var(--radius-sm)",
            letterSpacing: "0.03em",
          }}
        >
          {t("walkthrough.kind.save")}
        </span>
        <span style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--color-text)" }}>
          {item.text}
        </span>
      </div>
    );
  }

  if (item.kind === "ending") {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          margin: "var(--space-1) 0",
          padding: "var(--space-1) var(--space-2)",
        }}
      >
        <span
          style={{
            display: "inline-block",
            fontSize: "var(--font-size-xs)",
            fontWeight: 700,
            color: "var(--accent)",
            background: "var(--accent-soft)",
            padding: "2px 6px",
            borderRadius: "var(--radius-sm)",
            letterSpacing: "0.03em",
          }}
        >
          {t("walkthrough.kind.ending")}
        </span>
        <span style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--color-text)" }}>
          {item.text}
        </span>
      </div>
    );
  }

  if (item.kind === "date") {
    return (
      <div
        style={{
          fontSize: "var(--font-size-xs)",
          color: "var(--color-text-dim)",
          fontWeight: 600,
          padding: "var(--space-1) var(--space-2)",
          borderTop: "1px dashed var(--color-border)",
          marginTop: "var(--space-1)",
        }}
      >
        {item.text}
      </div>
    );
  }

  if (item.kind === "action") {
    return (
      <div
        style={{
          fontSize: "var(--font-size-xs)",
          color: "var(--color-text-dim)",
          padding: "2px var(--space-2)",
          fontStyle: "italic",
        }}
      >
        ↳ {item.text}
      </div>
    );
  }

  return (
    <div
      style={{
        fontSize: "var(--font-size-xs)",
        color: "var(--color-text-dim)",
        padding: "2px var(--space-2)",
      }}
    >
      {item.text}
    </div>
  );
}
