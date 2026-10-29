import { useCallback, useEffect, useMemo, useState } from "react";

import { useAppStore } from "@/app/store";
import { useI18n } from "@/app/useI18n";
import { Button } from "@/components/Button";
import { Card, CardHeader } from "@/components/Card";
import { Chip } from "@/components/Chip";
import { EmptyState } from "@/components/EmptyState";
import { Icon } from "@/components/Icon";
import { api } from "@/lib/ipc/client";
import type {
  ChapterCapabilityView,
  ManualTimerView,
  RouteView,
} from "@/lib/ipc/types";
import { formatDuration } from "@/lib/time";

/**
 * Routes, with their chapters and endings under them.
 *
 * This replaces a chapter panel and an ending panel that sat in different columns
 * and grouped the same story two different ways: chapters by an engine key, endings
 * by a free-text label, with nothing tying the halves together. One route row per
 * story is the whole point, so there is one card.
 *
 * The manual timer lives here rather than in a section of its own because it is the
 * same question the chapter rows answer -- which chapter is this time for -- and it
 * was the reason the chapter list and the "add time" button were separated before.
 */
export function RoutesCard({
  gameId,
  onAddTime,
}: {
  gameId: string;
  /** Opens the "add a past session" sheet. */
  onAddTime?: () => void;
}) {
  const { t, locale } = useI18n();
  const pushToast = useAppStore((state) => state.pushToast);

  const [routes, setRoutes] = useState<RouteView[]>([]);
  const [timer, setTimer] = useState<ManualTimerView | null>(null);
  const [capability, setCapability] = useState<ChapterCapabilityView | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [selectedChapterId, setSelectedChapterId] = useState<string | null>(null);
  const [addingRoute, setAddingRoute] = useState(false);
  const [newRouteName, setNewRouteName] = useState("");
  const [addingEndingTo, setAddingEndingTo] = useState<string | null>(null);
  const [endingName, setEndingName] = useState("");
  const [addingChapterTo, setAddingChapterTo] = useState<string | null>(null);
  const [chapterName, setChapterName] = useState("");
  const [renamingChapter, setRenamingChapter] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renamingRoute, setRenamingRoute] = useState<string | null>(null);
  const [routeRenameValue, setRouteRenameValue] = useState("");

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const list = await api.gameRoutes(gameId);
      setRoutes(list);
      setLoadError(null);
      setSelectedChapterId((current) => {
        if (current) {
          const stillThere = list.some((route) =>
            route.chapters.some((chapter) => chapter.id === current),
          );
          if (stillThere) return current;
        }
        return list.flatMap((route) => route.chapters)[0]?.id ?? null;
      });
    } catch (error) {
      // Do not blank the list the user is looking at, but do not hide the failure
      // either: silence here once made a broken request look like an empty library.
      setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoaded(true);
    }
  }, [gameId]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [list, state, capability] = await Promise.all([
          api.gameRoutes(gameId),
          api.trackingManualStatus(),
          api.trackingChapterCapability(gameId),
        ]);
        if (cancelled) return;
        setRoutes(list);
        setTimer(state);
        setCapability(capability);
        setSelectedChapterId(list.flatMap((route) => route.chapters)[0]?.id ?? null);
      } catch (error) {
        if (!cancelled) {
          setLoadError(error instanceof Error ? error.message : String(error));
        }
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [gameId]);

  // One interval for the whole card rather than per-row, so a long route list costs
  // nothing extra while a timer runs.
  useEffect(() => {
    if (!timer?.active || !timer.running) return;
    const id = window.setInterval(() => {
      void api
        .trackingManualStatus()
        .then(setTimer)
        .catch(() => undefined);
    }, 1000);
    return () => window.clearInterval(id);
  }, [timer?.active, timer?.running]);

  /**
   * The card's one honest sentence about automatic detection.
   *
   * Three outcomes, not two: detection works, the platform cannot read anything, or
   * the platform could but this build has no verified profile. Collapsing the last
   * two into "promises detection" is how a KiriKiri game sat on an empty list
   * waiting for something that was never coming.
   */
  const intro = useMemo(() => {
    if (!capability) return t("chapters.introManualOnly");
    if (capability.automatic) return t("chapters.intro");
    switch (capability.unavailableReason) {
      case "noExecutable":
        return t("chapters.introNoExecutable");
      case "engineHasNoVerifiedSource":
        return t("chapters.introNoProfile", {
          engine: capability.engine ?? t("chapters.introUnknownEngine"),
        });
      default:
        return t("chapters.introManualOnly");
    }
  }, [capability, t]);

  const run = useCallback(
    async (action: () => Promise<unknown>, message?: string): Promise<void> => {
      setBusy(true);
      try {
        await action();
        if (message) pushToast("success", message);
        await refresh();
      } catch (error) {
        pushToast("error", error instanceof Error ? error.message : String(error));
      } finally {
        setBusy(false);
      }
    },
    [pushToast, refresh],
  );

  const startTimer = useCallback(async (): Promise<void> => {
    if (!selectedChapterId) return;
    setBusy(true);
    try {
      setTimer(await api.trackingManualStart(selectedChapterId, gameId));
    } catch (error) {
      pushToast("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [gameId, pushToast, selectedChapterId]);

  /** Start timing a whole route, with no chapter named. */
  const startRouteTimer = useCallback(
    async (routeId: string): Promise<void> => {
      setBusy(true);
      try {
        setTimer(await api.trackingManualStartRoute(routeId, gameId));
      } catch (error) {
        pushToast("error", error instanceof Error ? error.message : String(error));
      } finally {
        setBusy(false);
      }
    },
    [gameId, pushToast],
  );

  const stopTimer = useCallback(async (): Promise<void> => {
    const name =
      routes
        .flatMap((route) => route.chapters)
        .find((chapter) => chapter.id === timer?.chapterId)?.displayName ??
      timer?.chapterName ??
      "";
    const elapsed = formatDuration(timer?.elapsedMs ?? 0, locale);
    setBusy(true);
    try {
      setTimer(await api.trackingManualStop());
      pushToast("success", t("manual.saved", { name, time: elapsed }));
      await refresh();
    } catch (error) {
      pushToast("error", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }, [locale, pushToast, refresh, routes, t, timer]);

  const totalChapters = routes.reduce((sum, route) => sum + route.chaptersTotal, 0);

  // The chapter the Start button would time. Naming it in the button is the whole
  // point: "Start" beside a list of chapters does not say which one it starts, and
  // picking the wrong row silently records time against the wrong chapter.
  const selectedChapterName =
    routes
      .flatMap((route) => route.chapters)
      .find((chapter) => chapter.id === selectedChapterId)?.displayName ?? null;

  const removeEnding = useCallback(
    (endingId: string) => {
      void run(() => api.deleteEnding(endingId), t("endings.removed"));
    },
    [run, t],
  );

  return (
    <Card>
      <CardHeader
        title={t("routes.title")}
        aside={
          <div className="chapter-head__actions">
            {onAddTime ? (
              <Button variant="ghost" onClick={onAddTime}>
                {t("detail.addSession")}
              </Button>
            ) : null}
            <Button
              variant="ghost"
              onClick={() => {
                setAddingRoute((open) => !open);
              }}
            >
              <Icon name="plus" />
              {t("routes.add")}
            </Button>
          </div>
        }
      />

      <p className="card__intro">{intro}</p>

      {addingRoute ? (
        <form
          className="chapter-form"
          onSubmit={(event) => {
            event.preventDefault();
            const name = newRouteName.trim();
            if (!name) return;
            void run(async () => {
              await api.createRoute(gameId, name);
              setNewRouteName("");
              setAddingRoute(false);
            });
          }}
        >
          <label className="field">
            <span className="field__label">{t("routes.nameLabel")}</span>
            <input
              className="input"
              value={newRouteName}
              onChange={(event) => {
                setNewRouteName(event.target.value);
              }}
              placeholder={t("chapters.routePlaceholder")}
              autoFocus
            />
          </label>
          <div className="chapter-form__actions">
            <Button type="submit" variant="primary" disabled={busy || !newRouteName.trim()}>
              {t("routes.create")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setAddingRoute(false);
                setNewRouteName("");
              }}
            >
              {t("routes.cancel")}
            </Button>
          </div>
        </form>
      ) : null}

      {!loaded ? (
        <div className="skeleton skeleton--row" />
      ) : loadError ? (
        <EmptyState title={t("routes.loadFailed")} body={loadError} />
      ) : routes.length === 0 ? (
        <EmptyState title={t("routes.empty")} body={t("routes.emptyBody")} />
      ) : (
        <div className="chapter-groups">
          {routes.map((route) => {
            const key = route.id ?? "unlinked";
            const isUnlinked = route.id === null;
            // The route timer is running for this section, rather than for one of its
            // chapters. Distinguished by `routeId`: a chapter-timed run has none.
            const routeTimerHere =
              timer?.active === true && route.id !== null && timer.routeId === route.id;
            // `route.id` is null for the unlinked bucket, and `renamingRoute` starts
            // as null, so a plain `===` matched every unlinked route on first render
            // and replaced its heading with an empty rename form.
            const isRenamingThisRoute = route.id !== null && renamingRoute === route.id;
            return (
              <section key={key} className="chapter-group">
                {/* The route is the unit; its chapters and endings are the detail. */}
                <h3 className="chapter-group__title">
                  {isRenamingThisRoute ? (
                    <form
                      className="chapter-row__rename"
                      onSubmit={(event) => {
                        event.preventDefault();
                        const name = routeRenameValue.trim();
                        if (!name || !route.id) return;
                        void run(async () => {
                          await api.renameRoute(route.id as string, name);
                          setRenamingRoute(null);
                        });
                      }}
                    >
                      <input
                        className="input input--compact"
                        value={routeRenameValue}
                        onChange={(event) => {
                          setRouteRenameValue(event.target.value);
                        }}
                        aria-label={t("routes.rename")}
                        autoFocus
                      />
                      <Button type="submit" variant="primary" disabled={busy}>
                        {t("routes.renameSave")}
                      </Button>
                    </form>
                  ) : (
                    <span>
                      {isUnlinked ? t("routes.unlinked") : route.name}
                      {/* Say where the row came from, so the same story appearing
                          twice before a merge is explainable rather than a bug. */}
                      {route.origin === "engine" ? (
                        <Chip className="chip--quiet">{t("routes.originEngine")}</Chip>
                      ) : null}
                      {route.origin === "ending" ? (
                        <Chip className="chip--quiet">{t("routes.originEnding")}</Chip>
                      ) : null}
                    </span>
                  )}
                  <span className="chapter-group__total">
                    {route.playtimeMs > 0 ? formatDuration(route.playtimeMs, locale) : null}
                    <span className="chapter-group__count">
                      {route.chaptersTotal > 0
                        ? t("chapters.routeCount", {
                            reached: String(route.chaptersReached),
                            total: String(route.chaptersTotal),
                          })
                        : null}
                      {route.endingsTotal > 0
                        ? ` · ${t("endings.count", {
                            reached: String(route.endingsReached),
                            total: String(route.endingsTotal),
                          })}`
                        : null}
                    </span>
                  </span>
                </h3>

                <div className="chapter-head__actions">
                  {/* Time the route itself, without naming a chapter. Offered only on
                      a real route: the unlinked bucket has no row to attribute time
                      to, and a session with neither chapter nor route would be time
                      nobody can point at. */}
                  {!isUnlinked && route.id ? (
                    routeTimerHere ? (
                      <Button
                        variant="primary"
                        disabled={busy}
                        onClick={() => {
                          void stopTimer();
                        }}
                      >
                        {t("manual.stop")}
                      </Button>
                    ) : (
                      <Button
                        variant="secondary"
                        disabled={busy || (timer?.active ?? false)}
                        title={
                          timer?.active ? t("routes.startBlocked") : undefined
                        }
                        onClick={() => {
                          void startRouteTimer(route.id as string);
                        }}
                      >
                        <Icon name="play" size={12} />
                        {t("routes.start")}
                      </Button>
                    )
                  ) : null}
                  {/* Add chapter is offered in the unlinked bucket too: a library
                      with no routes yet would otherwise have no way to create the
                      first chapter, and `Start` needs one to point at. */}
                  <Button
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      setAddingChapterTo(key);
                      setChapterName("");
                    }}
                  >
                    {t("chapters.add")}
                  </Button>
                  {!isUnlinked ? (
                    <>
                      <Button
                        variant="ghost"
                        disabled={busy}
                        onClick={() => {
                          setRenamingRoute(route.id);
                          setRouteRenameValue(route.name);
                        }}
                      >
                        {t("routes.rename")}
                      </Button>
                      <Button
                        variant="ghost"
                        disabled={busy}
                        onClick={() => {
                          setAddingEndingTo(route.id);
                          setEndingName("");
                        }}
                      >
                        {t("endings.add")}
                      </Button>
                    </>
                  ) : null}
                </div>

                {addingChapterTo === key ? (
                  <form
                    className="chapter-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      const name = chapterName.trim();
                      if (!name) return;
                      void run(async () => {
                        // Passing the route id both links the chapter to this route
                        // and lets Rust take the route's name, so adding a chapter
                        // here never asks the user to retype a route they are
                        // already inside.
                        const id = await api.trackingCreateChapter(
                          gameId,
                          name,
                          null,
                          route.id,
                        );
                        setAddingChapterTo(null);
                        setChapterName("");
                        // Selected straight away: the point of adding a chapter is
                        // almost always to start timing it.
                        setSelectedChapterId(id);
                      });
                    }}
                  >
                    <label className="field">
                      <span className="field__label">{t("chapters.name")}</span>
                      <input
                        className="input"
                        value={chapterName}
                        onChange={(event) => {
                          setChapterName(event.target.value);
                        }}
                        placeholder={t("chapters.name")}
                        autoFocus
                      />
                    </label>
                    <div className="chapter-form__actions">
                      <Button
                        type="submit"
                        variant="primary"
                        disabled={busy || !chapterName.trim()}
                      >
                        {t("chapters.create")}
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => {
                          setAddingChapterTo(null);
                          setChapterName("");
                        }}
                      >
                        {t("chapters.cancel")}
                      </Button>
                    </div>
                  </form>
                ) : null}

                {addingEndingTo !== null && addingEndingTo === route.id ? (
                  <form
                    className="chapter-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      const name = endingName.trim();
                      if (!name) return;
                      void run(async () => {
                        const created = await api.createEnding({
                          gameId,
                          name,
                          route: null,
                          achieved: false,
                          achievedAt: null,
                        });
                        // `create_ending` does not take a route id, so the new row is
                        // linked in a second step. Use the returned id: ending names
                        // are not unique on purpose ("Bad End" is routine), so
                        // looking the row up by name could re-point an older one.
                        await api.setEndingRoute(created.meta.id, addingEndingTo);
                        setAddingEndingTo(null);
                        setEndingName("");
                      });
                    }}
                  >
                    <label className="field">
                      <span className="field__label">{t("endings.name")}</span>
                      <input
                        className="input"
                        value={endingName}
                        onChange={(event) => {
                          setEndingName(event.target.value);
                        }}
                        placeholder={t("endings.namePlaceholder")}
                        autoFocus
                      />
                    </label>
                    <div className="chapter-form__actions">
                      <Button
                        type="submit"
                        variant="primary"
                        disabled={busy || !endingName.trim()}
                      >
                        {t("endings.create")}
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => {
                          setAddingEndingTo(null);
                          setEndingName("");
                        }}
                      >
                        {t("endings.cancel")}
                      </Button>
                    </div>
                  </form>
                ) : null}

                {route.chapters.length > 0 ? (
                  <ul className="chapter-list">
                    {route.chapters.map((chapter) => {
                      const isTimed =
                        timer?.chapterId === chapter.id && timer?.active;
                      const isSelected = selectedChapterId === chapter.id;
                      const isRenamingChapter = renamingChapter === chapter.id;
                      return (
                        <li
                          key={chapter.id}
                          className={`chapter-row${isSelected ? " chapter-row--selected" : ""}`}
                        >
                          <button
                            type="button"
                            className="chapter-row__pick"
                            aria-pressed={isSelected}
                            onClick={() => {
                              setSelectedChapterId(chapter.id);
                            }}
                          >
                            <span className="chapter-row__name">
                              {chapter.displayName}
                            </span>
                            <span className="chapter-row__meta">
                              {chapter.renamed ? (
                                <Chip>{t("chapters.renamed")}</Chip>
                              ) : null}
                              {chapter.renamed ? (
                                <span className="caption">
                                  {t("chapters.detectedName", { name: chapter.rawName })}
                                </span>
                              ) : null}
                              {chapter.manual ? (
                                <span className="caption">{t("chapters.manual")}</span>
                              ) : null}
                              <span className="caption tnum">
                                {chapter.playtimeMs > 0
                                  ? t("chapters.time", {
                                      time: formatDuration(chapter.playtimeMs, locale),
                                    })
                                  : t("chapters.untimed")}
                              </span>
                              {isTimed ? (
                                <Chip className="chip--accent">
                                  {formatDuration(timer?.elapsedMs ?? 0, locale)}
                                </Chip>
                              ) : null}
                            </span>
                          </button>

                          {isRenamingChapter ? (
                            <form
                              className="chapter-row__rename"
                              onSubmit={(event) => {
                                event.preventDefault();
                                const value = renameValue.trim();
                                void run(async () => {
                                  await api.trackingRenameChapter(
                                    chapter.id,
                                    value.length > 0 ? value : null,
                                  );
                                  setRenamingChapter(null);
                                });
                              }}
                            >
                              <input
                                className="input input--compact"
                                value={renameValue}
                                onChange={(event) => {
                                  setRenameValue(event.target.value);
                                }}
                                aria-label={t("chapters.rename")}
                                autoFocus
                              />
                              <Button type="submit" variant="primary" disabled={busy}>
                                {t("chapters.renameSave")}
                              </Button>
                              <Button
                                type="button"
                                variant="ghost"
                                onClick={() => {
                                  void run(async () => {
                                    await api.trackingRenameChapter(chapter.id, null);
                                    setRenamingChapter(null);
                                  });
                                }}
                              >
                                {t("chapters.renameClear")}
                              </Button>
                            </form>
                          ) : (
                            <Button
                              variant="ghost"
                              onClick={() => {
                                setRenamingChapter(chapter.id);
                                setRenameValue(chapter.displayName);
                              }}
                            >
                              {t("chapters.rename")}
                            </Button>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                ) : null}

                {route.endings.length > 0 ? (
                  <ul className="chapter-list">
                    {route.endings.map((ending) => (
                      <li key={ending.meta.id} className="chapter-row chapter-row--ending">
                        <div className="similar-row__main">
                          <span className="chapter-row__name">
                            <Icon name="check" size={12} />
                            {ending.name}
                            {ending.achieved ? (
                              <Chip className="chip--accent">{t("endings.reached")}</Chip>
                            ) : null}
                          </span>
                          {ending.achieved_at !== null ? (
                            <span className="chapter-row__meta caption">
                              {new Date(ending.achieved_at).toLocaleDateString(undefined, {
                                year: "numeric",
                                month: "short",
                                day: "numeric",
                              })}
                            </span>
                          ) : null}
                        </div>
                        <div className="chapter-head__actions">
                          <Button
                            variant="ghost"
                            disabled={busy}
                            onClick={() => {
                              void run(async () => {
                                await api.updateEnding(ending.meta.id, {
                                  gameId,
                                  name: ending.name,
                                  route: ending.route,
                                  achieved: !ending.achieved,
                                  achievedAt: ending.achieved ? null : Date.now(),
                                });
                              });
                            }}
                          >
                            {ending.achieved ? t("endings.unmark") : t("endings.mark")}
                          </Button>
                          <Button
                            variant="ghost"
                            disabled={busy}
                            onClick={() => {
                              removeEnding(ending.meta.id);
                            }}
                          >
                            {t("endings.remove")}
                          </Button>
                        </div>
                      </li>
                    ))}
                  </ul>
                ) : null}

                {/* Two route rows for one story is the state the backfill creates on
                    purpose, because the link was not provable. Offer the merge. */}
                {!isUnlinked && route.mergeCandidates.length > 0 ? (
                  // One button per target, not one button that guesses. Which route
                  // survives is a decision the user cannot undo, so it is shown
                  // before the write rather than reported after it.
                  <div className="chapter-group__merge">
                    {route.mergeCandidates.map((candidateId) => {
                      const candidate = routes.find((other) => other.id === candidateId);
                      if (!candidate || !route.id) return null;
                      return (
                        <Button
                          key={candidateId}
                          variant="ghost"
                          disabled={busy}
                          onClick={() => {
                            void run(async () => {
                              await api.mergeRoutes(route.id ?? "", candidateId);
                            }, t("routes.merged", { into: candidate.name }));
                          }}
                        >
                          {t("routes.mergeInto", { into: candidate.name })}
                        </Button>
                      );
                    })}
                  </div>
                ) : null}
              </section>
            );
          })}
        </div>
      )}

      <div className="manual-timer">
        <div className="manual-timer__status">
          {timer?.active ? (
            <>
              <span className="manual-timer__clock">
                {formatDuration(timer.elapsedMs)}
              </span>
              <span className="manual-timer__label">
                {timer.running
                  ? t("manual.running", { name: timer.chapterName ?? "" })
                  : t("manual.paused", { time: formatDuration(timer.elapsedMs) })}
              </span>
            </>
          ) : (
            <span className="manual-timer__label">
              {totalChapters > 0 ? t("manual.idle") : t("manual.orCreate")}
            </span>
          )}
        </div>

        <div className="manual-timer__actions">
          {timer?.active ? (
            <>
              {timer.running ? (
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() => {
                    void (async () => {
                      setBusy(true);
                      try {
                        setTimer(await api.trackingManualPause());
                      } catch (error) {
                        pushToast(
                          "error",
                          error instanceof Error ? error.message : String(error),
                        );
                      } finally {
                        setBusy(false);
                      }
                    })();
                  }}
                >
                  {t("manual.pause")}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() => {
                    void (async () => {
                      setBusy(true);
                      try {
                        setTimer(await api.trackingManualResume());
                      } catch (error) {
                        pushToast(
                          "error",
                          error instanceof Error ? error.message : String(error),
                        );
                      } finally {
                        setBusy(false);
                      }
                    })();
                  }}
                >
                  {t("manual.resume")}
                </Button>
              )}
              <Button
                variant="primary"
                disabled={busy}
                onClick={() => {
                  void stopTimer();
                }}
              >
                {t("manual.stop")}
              </Button>
            </>
          ) : (
            <Button
              variant="primary"
              disabled={busy || !selectedChapterId}
              onClick={() => {
                void startTimer();
              }}
            >
              {/* The name is clipped by CSS rather than truncated in JS: a VN chapter
                  can be titled in Japanese with thirty characters, and counting
                  graphemes here to fit a box we do not measure would be guessing. */}
              <span className="manual-timer__start-label">
                {selectedChapterName
                  ? t("manual.startNamed", { name: selectedChapterName })
                  : t("manual.start")}
              </span>
            </Button>
          )}
        </div>

        <p className="manual-timer__hint">{t("manual.intro")}</p>
      </div>
    </Card>
  );
}
