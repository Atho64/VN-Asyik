/**
 * The only place the frontend talks to Rust.
 *
 * Every response is parsed by its zod schema before it reaches a component, so a
 * contract drift fails here with a readable error instead of rendering garbage.
 * Calls require the Tauri shell; tests mock the command boundary explicitly.
 */

import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

import {
  walkthroughViewSchema,
  walkthroughIndexEntrySchema,
  type WalkthroughView,
  type WalkthroughIndexEntry,
  androidAppSchema,
  androidSessionViewSchema,
  type AndroidApp,
  type AndroidSessionView,
  afkConfigSchema,
  applyResultSchema,
  chapterSchema,
  chapterCapabilityViewSchema,
  routeViewSchema,
  type RouteView,
  usageAccessViewSchema,
  syncStatusSchema,
  syncOutcomeSchema,
  conflictViewSchema,
  tasteViewSchema,
  chatSettingsSchema,
  recommendSettingsSchema,
  recommendOutcomeSchema,
  launchCapabilitySchema,
  launchOutcomeSchema,
  titleConfigSchema,
  statsPayloadSchema,
  patternTestSchema,
  bulkImportSchema,
  refreshAllSchema,
  searchResponseSchema,
  chapterViewSchema,
  detailPayloadSchema,
  importOutcomeSchema,
  localArtSchema,
  manualTimerViewSchema,
  logEntrySchema,
  pullResultSchema,
  pushResultSchema,
  queueEntrySchema,
  screenshotGallerySchema,
  screenshotViewSchema,
  syncSettingsSchema,
  executableCandidateSchema,
  gameExecutableSchema,
  trackerCapabilitySchema,
  trackingStatusSchema,
  ulistLabelInfoSchema,
  vndbAccountSchema,
  vndbStatusSchema,
  endingSchema,
  gameCardSchema,
  gameSchema,
  importSummarySchema,
  dataImportPreviewSchema,
  dataImportResultSchema,
  platformInfoSchema,
  gameSessionsPageSchema,
  playSessionSchema,
  prefsSchema,
  statusBreakdownSchema,
  type AfkConfig,
  type Chapter,
  type ChapterCapabilityView,
  type GameSessionsPage,
  type UsageAccessView,
  type SyncStatus,
  type SyncOutcome,
  type ConflictView,
  type TasteView,
  type ChatSettings,
  type RecommendSettings,
  type RecommendOutcome,
  type LaunchCapability,
  type LaunchOutcome,
  type TitleConfig,
  type StatsPayload,
  type PatternTest,
  type BulkImportOutcome,
  type MissingEntry,
  type DetailPayload,
  type ScreenshotGallery,
  type ScreenshotView,
  type SearchResponse,
  type ImportOutcome,
  type RefreshAllOutcome,
  type EndingInput,
  type ApplyResult,
  type ConflictPolicy,
  type LabelMapping,
  type LocalArt,
  type LogEntry,
  type PullResult,
  type PushResult,
  type QueueEntry,
  type SyncSettings,
  type ChapterView,
  type ExecutableCandidate,
  type GameExecutable,
  type MapRunnerTargetInput,
  type ManualTimerView,
  type TrackerCapability,
  type TrackingStatus,
  type UlistLabelInfo,
  type VndbAccount,
  type VndbStatus,
  type Ending,
  type Game,
  type GameCard,
  type ImportSummary,
  type DataImportPreview,
  type DataImportResult,
  type PlatformInfo,
  type PlaySession,
  type Prefs,
  type StatusBreakdown,
  presenceActivitySchema,
  presenceSettingsSchema,
  presenceStatusSchema,
  type PresenceActivity,
  type PresenceSettings,
  type PresenceStatus,
} from "./types";

/** A command failure the UI can display directly. */
export class IpcError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "IpcError";
    this.code = code;
  }
}

const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function call<T>(
  command: string,
  schema: z.ZodType<T>,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    const raw = await invoke<unknown>(command, args);
    const parsed = schema.safeParse(raw);
    if (!parsed.success) {
      throw new IpcError(
        "contract",
        `Unexpected response from ${command}: ${parsed.error.message}`,
      );
    }
    return parsed.data;
  } catch (error) {
    if (error instanceof IpcError) throw error;
    if (error && typeof error === "object" && "code" in error && "message" in error) {
      const shaped = error as { code: unknown; message: unknown };
      throw new IpcError(String(shaped.code), String(shaped.message));
    }
    throw new IpcError("ipc", error instanceof Error ? error.message : String(error));
  }
}

/** Requests already on the wire, keyed by command. */
const inFlight = new Map<string, Promise<unknown>>();

/**
 * Share one in-flight request between callers of the same command.
 *
 * Only for calls whose answer does not depend on arguments: the library totals
 * and the tracking status. A caller that starts while the previous answer is
 * still travelling gets that answer instead of paying for a second aggregate.
 * The entry is cleared as soon as it settles, so the next call is fresh.
 */
function shared<T>(key: string, run: () => Promise<T>): Promise<T> {
  const pending = inFlight.get(key);
  if (pending) return pending as Promise<T>;
  const request = run().finally(() => inFlight.delete(key));
  inFlight.set(key, request);
  return request;
}

/** Result of a filtered, sorted library query. */
export interface LibraryPayload {
  games: GameCard[];
  prefs: Prefs;
}

export interface CreateGameInput {
  title: string;
  status?: string;
}

export interface UpdateGameInput {
  id: string;
  title?: string;
  status?: string;
  rating?: number | null;
  notes?: string | null;
  manual_playtime_ms?: number;
  vndb_id?: string;
}

export interface ManualSessionInput {
  gameId: string;
  startedAtMs: number;
  endedAtMs: number;
}

/** The command surface, one method per Rust command. */
export interface Api {
  listGames(): Promise<GameCard[]>;
  getGame(id: string): Promise<GameCard | null>;
  createGame(input: CreateGameInput): Promise<Game>;
  updateGame(input: UpdateGameInput): Promise<Game>;
  deleteGame(id: string): Promise<void>;
  listChapters(gameId: string): Promise<Chapter[]>;
  listSessions(gameId: string): Promise<PlaySession[]>;
  /** Newest sessions plus exact totals over all of them. */
  listGameSessions(gameId: string): Promise<GameSessionsPage>;
  listEndings(gameId: string): Promise<Ending[]>;
  gameRoutes(gameId: string): Promise<RouteView[]>;
  createRoute(gameId: string, name: string): Promise<string>;
  renameRoute(routeId: string, name: string): Promise<void>;
  mergeRoutes(sourceId: string, targetId: string): Promise<void>;
  setChapterRoute(chapterId: string, routeId: string | null): Promise<void>;
  setEndingRoute(endingId: string, routeId: string | null): Promise<void>;
  createEnding(input: EndingInput): Promise<Ending>;
  updateEnding(endingId: string, input: EndingInput): Promise<Ending>;
  deleteEnding(endingId: string): Promise<void>;
  addManualSession(input: ManualSessionInput): Promise<PlaySession>;
  statusBreakdown(): Promise<StatusBreakdown>;
  playtimeTotals(): Promise<[number, number]>;
  playtimeByDay(): Promise<Array<[number, number]>>;
  getPrefs(): Promise<Prefs>;
  setTheme(value: string): Promise<Prefs>;
  setLanguage(value: string): Promise<Prefs>;
  setNsfwMode(value: string): Promise<Prefs>;
  setReduceEffects(value: boolean): Promise<Prefs>;
  getAfkConfig(): Promise<AfkConfig>;
  setAfkConfig(thresholdMs: number, excludeAfk: boolean): Promise<AfkConfig>;
  vndbSearch(query: string, page?: number): Promise<SearchResponse>;
  vndbPreview(vnId: string): Promise<DetailPayload>;
  vndbRefreshDetail(gameId: string): Promise<DetailPayload>;
  vndbRefreshAll(startAt?: number): Promise<RefreshAllOutcome>;
  vndbImport(vnId: string): Promise<ImportOutcome>;
  vndbImportMany(entries: MissingEntry[]): Promise<BulkImportOutcome>;
  vndbDetail(vnId: string): Promise<DetailPayload | null>;
  vndbLocalArt(vnId: string): Promise<LocalArt>;
  /** A game's user-chosen cover art, keyed by game id rather than VN id. */
  gameCustomArt(id: string): Promise<LocalArt>;
  /** Import an image file as this game's cover. */
  setGameCustomCover(id: string, path: string): Promise<Game>;
  /** Remove the custom cover and fall back to the previous artwork. */
  clearGameCustomCover(id: string): Promise<Game>;
  /**
   * Screenshot gallery, cached on demand.
   *
   * Policy-free on purpose: like {@link vndbLocalArt}, the payload carries both the
   * sharp and the pre-blurred copy of every tile and the surface decides which to
   * paint. That is what lets a reveal paint instantly instead of re-fetching, and it
   * is why changing the content setting no longer needs a round trip.
   */
  vndbScreenshots(vnId: string): Promise<ScreenshotGallery>;
  /** One screenshot at full cached size, for the lightbox. */
  vndbScreenshotFetch(vnId: string, imageId: string): Promise<ScreenshotView>;
  setGameAccent(id: string, accent: string): Promise<Game>;
  vndbStatus(): Promise<VndbStatus>;
  vndbConnect(token: string): Promise<VndbAccount>;
  vndbDisconnect(): Promise<void>;
  vndbTokenHint(): Promise<string | null>;
  vndbSyncSettings(): Promise<SyncSettings>;
  vndbSaveSyncSettings(
    mapping: LabelMapping,
    policy: ConflictPolicy,
  ): Promise<SyncSettings>;
  vndbPull(): Promise<PullResult>;
  vndbApplySync(input: {
    decisions: Array<{
      vn_id: string;
      fields: Array<{ field: string; resolution: string }>;
    }>;
    push_all_local: boolean;
  }): Promise<ApplyResult>;
  vndbPushPending(): Promise<PushResult>;
  vndbQueue(): Promise<QueueEntry[]>;
  vndbLog(): Promise<LogEntry[]>;
  vndbLabels(): Promise<UlistLabelInfo[]>;
  trackingCapability(): Promise<TrackerCapability>;
  trackingStatus(): Promise<TrackingStatus>;
  trackingSetEnabled(enabled: boolean): Promise<TrackerCapability>;
  trackingMapExecutable(gameId: string, exePath: string): Promise<GameExecutable>;
  trackingMapRunnerTarget(input: MapRunnerTargetInput): Promise<GameExecutable>;
  trackingListExecutables(): Promise<GameExecutable[]>;
  trackingRemoveExecutable(id: string): Promise<void>;
  trackingListChapters(gameId: string): Promise<ChapterView[]>;
  trackingChapterCapability(gameId?: string): Promise<ChapterCapabilityView>;
  trackingUsageAccess(): Promise<UsageAccessView>;
  syncStatus(): Promise<SyncStatus>;
  syncRun(folder: string): Promise<SyncOutcome>;
  syncConflicts(limit: number): Promise<ConflictView[]>;
  aiTasteProfile(): Promise<TasteView>;
  aiChatSettings(): Promise<ChatSettings>;
  aiSaveChatSettings(input: {
    baseUrl: string;
    model: string;
    key?: string;
  }): Promise<ChatSettings>;
  aiClearChatKey(): Promise<ChatSettings>;
  aiRecommendSettings(): Promise<RecommendSettings>;
  aiSaveTavilyKey(key: string): Promise<RecommendSettings>;
  aiClearTavilyKey(): Promise<RecommendSettings>;
  aiRecommend(request: string, depth: string): Promise<RecommendOutcome>;
  launchCapability(): Promise<LaunchCapability>;
  launchSaveSettings(input: {
    launcherDir?: string | null;
    guid?: string | null;
    /** Omit to leave the current on/off state untouched. */
    emulatorEnabled?: boolean;
  }): Promise<LaunchCapability>;
  launchGame(gameId: string): Promise<LaunchOutcome>;
  statsDerived(period: string): Promise<StatsPayload>;
  trackingTitleConfig(): Promise<TitleConfig>;
  trackingSaveTitleConfig(format: string, pattern: string | null): Promise<TitleConfig>;
  trackingTestTitlePattern(pattern: string, titles: string[]): Promise<PatternTest[]>;
  trackingOpenUsageSettings(): Promise<void>;
  trackingAndroidApps(): Promise<AndroidApp[]>;
  trackingAndroidStatus(): Promise<AndroidSessionView>;
  trackingAndroidSession(): Promise<AndroidSessionView>;
  trackingAndroidStop(): Promise<AndroidSessionView>;
  syncExportFile(): Promise<boolean>;
  syncImportFile(): Promise<SyncOutcome | null>;
  syncShareFile(): Promise<void>;
  trackingRenameChapter(chapterId: string, displayName: string | null): Promise<void>;
  trackingCreateChapter(
    gameId: string,
    name: string,
    routeName: string | null,
    routeId?: string | null,
  ): Promise<string>;
  trackingManualStatus(): Promise<ManualTimerView>;
  trackingManualStart(chapterId: string, gameId: string): Promise<ManualTimerView>;
  trackingManualStartRoute(routeId: string, gameId: string): Promise<ManualTimerView>;
  trackingManualPause(): Promise<ManualTimerView>;
  trackingManualResume(): Promise<ManualTimerView>;
  trackingManualStop(): Promise<ManualTimerView>;
  trackingPause(): Promise<void>;
  trackingResume(): Promise<void>;
  trackingStop(): Promise<void>;
  trackingListScanFolders(): Promise<string[]>;
  trackingAddScanFolder(path: string): Promise<void>;
  trackingRemoveScanFolder(path: string): Promise<void>;
  trackingScanFolder(path: string): Promise<ExecutableCandidate[]>;
  exportBackup(): Promise<unknown>;
  importBackup(payload: unknown): Promise<ImportSummary>;
  exportBackupFile(): Promise<boolean>;
  importBackupFile(): Promise<ImportSummary | null>;
  shareBackupFile(): Promise<void>;
  dataOpenFile(): Promise<DataImportPreview | null>;
  dataExportFile(): Promise<boolean>;
  dataShareFile(): Promise<void>;
  dataApplyImport(id: string, mode?: "merge" | "restore"): Promise<DataImportResult>;
  dataCancelImport(id: string): Promise<void>;
  platformInfo(): Promise<PlatformInfo>;
  presenceStatus(): Promise<PresenceStatus>;
  presenceGetSettings(): Promise<PresenceSettings>;
  presenceSaveSettings(settings: PresenceSettings, gameId?: string): Promise<void>;
  presenceEffectiveSettings(gameId?: string): Promise<PresenceSettings>;
  presenceResetGameSettings(gameId: string): Promise<PresenceSettings>;
  presenceSetEnabled(enabled: boolean): Promise<PresenceSettings>;
  presenceClear(): Promise<void>;
  /**
   * Render the activity the given settings would produce.
   *
   * `cycleIndex` / `stateCycleIndex` step the preview through a rotation list, so
   * a user can check entry 3 of a template list without saving first.
   */
  presencePreview(
    settings: PresenceSettings,
    gameId?: string,
    cycleIndex?: number,
    stateCycleIndex?: number,
  ): Promise<PresenceActivity>;
  presenceHasToken(): Promise<boolean>;
  presenceSaveToken(token: string): Promise<void>;
  presenceClearToken(): Promise<void>;
  presenceTokenPreview(): Promise<string | null>;
  /** imgbb API key for the presence image host, in the OS credential store. */
  presenceImageKeySave(key: string): Promise<void>;
  presenceImageKeyClear(): Promise<void>;
  presenceImageKeyPreview(): Promise<string | null>;
  /** Open a URL in the OS default browser. Validated on the Rust side. */
  openExternal(url: string): Promise<void>;
  walkthroughGet(gameId: string, vnId?: string | null): Promise<WalkthroughView | null>;
  walkthroughSearchCandidate(gameId: string, vnId?: string | null): Promise<WalkthroughIndexEntry[]>;
  walkthroughSearch(query: string): Promise<WalkthroughIndexEntry[]>;
  walkthroughFetch(gameId: string, vnId?: string | null, url?: string | null): Promise<WalkthroughView>;
  walkthroughTranslate(
    gameId: string,
    provider: "ai" | "google",
    targetLang: "en" | "id",
    vnId?: string | null,
  ): Promise<WalkthroughView>;
  walkthroughToggleItem(gameId: string, itemId: string): Promise<string[]>;
  walkthroughClearProgress(gameId: string): Promise<void>;
  hideWindow(): Promise<void>;
  minimizeWindow(): Promise<void>;
  showWindow(): Promise<void>;
  toggleWindow(): Promise<boolean>;
  quitApp(): Promise<void>;
}

/** `void` commands still resolve to `null` over IPC. */
const voidSchema = z.unknown().transform(() => undefined as void);

/**
 * Run `run` immediately, then every `intervalMs`, waiting for each call to settle.
 *
 * A fixed `setInterval` starts a new request whether or not the last one came
 * back. A slow IPC round trip (an Android usage query, a cold database open) then
 * stacks requests that are answered out of order, and the oldest answer can
 * overwrite the newest state. This keeps exactly one request in flight.
 */
export function startPolling(
  run: () => Promise<void>,
  intervalMs: number,
): () => void {
  let stopped = false;
  let timer = 0;
  const tick = async (): Promise<void> => {
    if (stopped) return;
    try {
      await run();
    } catch {
      // The caller decides how to report a failure; polling continues.
    } finally {
      if (!stopped) timer = window.setTimeout(() => void tick(), intervalMs);
    }
  };
  void tick();
  return () => {
    stopped = true;
    window.clearTimeout(timer);
  };
}

/**
 * Drop a response once its caller has gone away.
 *
 * Pollers answer every few seconds, and a slow round trip can outlive the
 * screen that asked for it. This keeps scheduling the next tick only while a
 * request is still in flight, so a stalled backend stops accumulating
 * overlapping calls instead of piling one up per interval.
 */
export function polled<T>(
  promise: Promise<T>,
  apply: (value: T) => void,
  isActive: () => boolean,
): Promise<void> {
  return promise.then(
    (value) => {
      if (isActive()) apply(value);
    },
    () => undefined,
  );
}

export const api: Api = {
  // The library totals are recomputed by `list_games` on every call, so a second
  // concurrent request (boot, focus refresh, screen reload) would pay a full
  // session aggregate again. Callers share one in-flight answer instead.
  listGames: () => shared("list_games", () => call("list_games", gameCardSchema.array())),
  getGame: (id) => call("get_game", gameCardSchema.nullable(), { id }),
  createGame: (input) => call("create_game", gameSchema, { input }),
  updateGame: (input) => call("update_game", gameSchema, { input }),
  deleteGame: (id) => call("delete_game", voidSchema, { id }),
  listChapters: (gameId) => call("list_chapters", chapterSchema.array(), { gameId }),
  listSessions: (gameId) => call("list_sessions", playSessionSchema.array(), { gameId }),
  listGameSessions: (gameId) =>
    call("list_game_sessions", gameSessionsPageSchema, { gameId }),
  listEndings: (gameId) => call("list_endings", endingSchema.array(), { gameId }),
  gameRoutes: (gameId) => call("game_routes", routeViewSchema.array(), { gameId }),
  createRoute: (gameId, name) => call("create_route", z.string(), { gameId, name }),
  renameRoute: (routeId, name) => call("rename_route", voidSchema, { routeId, name }),
  mergeRoutes: (sourceId, targetId) =>
    call("merge_routes", voidSchema, { sourceId, targetId }),
  setChapterRoute: (chapterId, routeId) =>
    call("set_chapter_route", voidSchema, { chapterId, routeId }),
  setEndingRoute: (endingId, routeId) =>
    call("set_ending_route", voidSchema, { endingId, routeId }),
  createEnding: (input) =>
    call("create_ending", endingSchema, {
      gameId: input.gameId,
      name: input.name,
      route: input.route,
      achieved: input.achieved,
      achievedAtMs: input.achievedAt,
    }),
  updateEnding: (endingId, input) =>
    call("update_ending", endingSchema, {
      endingId,
      name: input.name,
      route: input.route,
      achieved: input.achieved,
      achievedAtMs: input.achievedAt,
    }),
  deleteEnding: (endingId) => call("delete_ending", voidSchema, { endingId }),
  addManualSession: (input) =>
    call("add_manual_session", playSessionSchema, {
      input: {
        game_id: input.gameId,
        started_at_ms: input.startedAtMs,
        ended_at_ms: input.endedAtMs,
      },
    }),
  statusBreakdown: () => call("status_breakdown", statusBreakdownSchema),
  playtimeTotals: () => call("playtime_totals", z.tuple([z.number(), z.number()])),
  playtimeByDay: () =>
    call("playtime_by_day", z.array(z.tuple([z.number(), z.number()]))),
  getPrefs: () => call("get_prefs", prefsSchema),
  setTheme: (value) => call("set_theme", prefsSchema, { value }),
  setLanguage: (value) => call("set_language", prefsSchema, { value }),
  setNsfwMode: (value) => call("set_nsfw_mode", prefsSchema, { value }),
  setReduceEffects: (value) => call("set_reduce_effects", prefsSchema, { value }),
  getAfkConfig: () => call("get_afk_config", afkConfigSchema),
  setAfkConfig: (thresholdMs, excludeAfk) =>
    call("set_afk_config", afkConfigSchema, {
      thresholdMs,
      excludeAfk,
    }),
  vndbSearch: (query, page) => call("vndb_search", searchResponseSchema, { query, page }),
  vndbPreview: (vnId) => call("vndb_preview", detailPayloadSchema, { vnId }),
  vndbRefreshDetail: (gameId) =>
    call("vndb_refresh_detail", detailPayloadSchema, { gameId }),
  vndbRefreshAll: (startAt) =>
    call("vndb_refresh_all", refreshAllSchema, { startAt: startAt ?? null }),
  vndbImport: (vnId) => call("vndb_import", importOutcomeSchema, { vnId }),
  vndbImportMany: (entries) => call("vndb_import_many", bulkImportSchema, { entries }),
  vndbDetail: (vnId) => call("vndb_detail", detailPayloadSchema.nullable(), { vnId }),
  vndbLocalArt: (vnId) => call("vndb_local_art", localArtSchema, { vnId }),
  gameCustomArt: (id) => call("game_custom_art", localArtSchema, { id }),
  setGameCustomCover: (id, path) =>
    call("set_game_custom_cover", gameSchema, { id, path }),
  clearGameCustomCover: (id) => call("clear_game_custom_cover", gameSchema, { id }),
  vndbScreenshots: (vnId) => call("vndb_screenshots", screenshotGallerySchema, { vnId }),
  vndbScreenshotFetch: (vnId, imageId) =>
    call("vndb_screenshot_fetch", screenshotViewSchema, { vnId, imageId }),
  setGameAccent: (id, accent) => call("set_game_accent", gameSchema, { id, accent }),
  vndbStatus: () => call("vndb_status", vndbStatusSchema),
  vndbConnect: (token) => call("vndb_connect", vndbAccountSchema, { token }),
  vndbDisconnect: () => call("vndb_disconnect", voidSchema),
  vndbTokenHint: () => call("vndb_token_hint", z.string().nullable()),
  vndbSyncSettings: () => call("vndb_sync_settings", syncSettingsSchema),
  vndbSaveSyncSettings: (mapping, policy) =>
    call("vndb_save_sync_settings", syncSettingsSchema, { mapping, policy }),
  vndbPull: () => call("vndb_pull", pullResultSchema),
  vndbApplySync: (input) => call("vndb_apply_sync", applyResultSchema, { input }),
  vndbPushPending: () => call("vndb_push_pending", pushResultSchema),
  vndbQueue: () => call("vndb_queue", queueEntrySchema.array()),
  vndbLog: () => call("vndb_log", logEntrySchema.array()),
  vndbLabels: () => call("vndb_labels", ulistLabelInfoSchema.array()),
  trackingCapability: () => call("tracking_capability", trackerCapabilitySchema),
  // Polled every few seconds; coalesce so a slow tick cannot queue more work.
  trackingStatus: () =>
    shared("tracking_status", () => call("tracking_status", trackingStatusSchema)),
  trackingSetEnabled: (enabled) =>
    call("tracking_set_enabled", trackerCapabilitySchema, { enabled }),
  trackingMapExecutable: (gameId, exePath) =>
    call("tracking_map_executable", gameExecutableSchema, {
      input: { game_id: gameId, exe_path: exePath },
    }),
  trackingMapRunnerTarget: (input) =>
    call("tracking_map_runner_target", gameExecutableSchema, { input }),
  trackingListExecutables: () =>
    call("tracking_list_executables", gameExecutableSchema.array()),
  trackingRemoveExecutable: (id) =>
    call("tracking_remove_executable", voidSchema, { id }),
  trackingListChapters: (gameId) =>
    call("tracking_list_chapters", chapterViewSchema.array(), { gameId }),
  trackingChapterCapability: (gameId) =>
    call("tracking_chapter_capability", chapterCapabilityViewSchema, { gameId }),
  trackingUsageAccess: () => call("tracking_usage_access", usageAccessViewSchema),
  syncStatus: () => call("sync_status", syncStatusSchema),
  syncRun: (folder) => call("sync_run", syncOutcomeSchema, { folder }),
  syncConflicts: (limit) => call("sync_conflicts", conflictViewSchema.array(), { limit }),
  syncExportFile: () => call("sync_export_file", z.boolean()),
  syncImportFile: () => call("sync_import_file", syncOutcomeSchema.nullable()),
  syncShareFile: () => call("sync_share_file", voidSchema),
  trackingAndroidApps: () => call("tracking_android_apps", androidAppSchema.array()),
  trackingAndroidStatus: () => call("tracking_android_status", androidSessionViewSchema),
  trackingAndroidSession: () =>
    call("tracking_android_session", androidSessionViewSchema),
  trackingAndroidStop: () => call("tracking_android_stop", androidSessionViewSchema),
  aiTasteProfile: () => call("ai_taste_profile", tasteViewSchema),
  aiChatSettings: () => call("ai_chat_settings", chatSettingsSchema),
  aiSaveChatSettings: (input) =>
    call("ai_save_chat_settings", chatSettingsSchema, { input }),
  aiClearChatKey: () => call("ai_clear_chat_key", chatSettingsSchema),
  aiRecommendSettings: () => call("ai_recommend_settings", recommendSettingsSchema),
  aiSaveTavilyKey: (key) => call("ai_save_tavily_key", recommendSettingsSchema, { key }),
  aiClearTavilyKey: () => call("ai_clear_tavily_key", recommendSettingsSchema),
  aiRecommend: (request, depth) =>
    call("ai_recommend", recommendOutcomeSchema, { request, depth }),
  launchCapability: () => call("launch_capability", launchCapabilitySchema),
  launchSaveSettings: (input) =>
    call("launch_save_settings", launchCapabilitySchema, { input }),
  launchGame: (gameId) => call("launch_game", launchOutcomeSchema, { gameId }),
  statsDerived: (period) => call("stats_derived", statsPayloadSchema, { period }),
  trackingTitleConfig: () => call("tracking_title_config", titleConfigSchema),
  trackingSaveTitleConfig: (format, pattern) =>
    call("tracking_save_title_config", titleConfigSchema, { format, pattern }),
  trackingTestTitlePattern: (pattern, titles) =>
    call("tracking_test_title_pattern", patternTestSchema.array(), { pattern, titles }),
  trackingOpenUsageSettings: () => call("tracking_open_usage_settings", voidSchema),
  trackingRenameChapter: (chapterId, displayName) =>
    call("tracking_rename_chapter", voidSchema, { chapterId, displayName }),
  trackingCreateChapter: (gameId, name, routeName, routeId = null) =>
    call("tracking_create_chapter", z.string(), { gameId, name, routeName, routeId }),
  trackingManualStatus: () => call("tracking_manual_status", manualTimerViewSchema),
  trackingManualStart: (chapterId, gameId) =>
    call("tracking_manual_start", manualTimerViewSchema, { chapterId, gameId }),
  trackingManualStartRoute: (routeId, gameId) =>
    call("tracking_manual_start_route", manualTimerViewSchema, { routeId, gameId }),
  trackingManualPause: () => call("tracking_manual_pause", manualTimerViewSchema),
  trackingManualResume: () => call("tracking_manual_resume", manualTimerViewSchema),
  trackingManualStop: () => call("tracking_manual_stop", manualTimerViewSchema),
  trackingPause: () => call("tracking_pause", voidSchema),
  trackingResume: () => call("tracking_resume", voidSchema),
  trackingStop: () => call("tracking_stop", voidSchema),
  trackingListScanFolders: () => call("tracking_list_scan_folders", z.string().array()),
  trackingAddScanFolder: (path) => call("tracking_add_scan_folder", voidSchema, { path }),
  trackingRemoveScanFolder: (path) =>
    call("tracking_remove_scan_folder", voidSchema, { path }),
  trackingScanFolder: (path) =>
    call("tracking_scan_folder", executableCandidateSchema.array(), { path }),
  exportBackup: () => call("export_backup", z.unknown()),
  exportBackupFile: () => call("export_backup_file", z.boolean()),
  importBackupFile: () => call("import_backup_file", importSummarySchema.nullable()),
  shareBackupFile: () => call("share_backup_file", voidSchema),
  dataOpenFile: () => call("data_open_file", dataImportPreviewSchema.nullable()),
  dataExportFile: () => call("data_export_file", z.boolean()),
  dataShareFile: () => call("data_share_file", voidSchema),
  dataApplyImport: (id, mode = "merge") =>
    call("data_apply_import", dataImportResultSchema, { id, mode }),
  dataCancelImport: (id) => call("data_cancel_import", voidSchema, { id }),
  importBackup: (payload) => call("import_backup", importSummarySchema, { payload }),
  platformInfo: () => call("platform_info", platformInfoSchema),
  presenceStatus: () => call("presence_status", presenceStatusSchema),
  presenceGetSettings: () => call("presence_get_settings", presenceSettingsSchema),
  presenceSaveSettings: (settings, gameId) =>
    call("presence_save_settings", voidSchema, { settings, gameId }),
  presenceEffectiveSettings: (gameId) =>
    call("presence_effective_settings", presenceSettingsSchema, { gameId }),
  presenceResetGameSettings: (gameId) =>
    call("presence_reset_game_settings", presenceSettingsSchema, { gameId }),
  presenceSetEnabled: (enabled) =>
    call("presence_set_enabled", presenceSettingsSchema, { enabled }),
  presenceClear: () => call("presence_clear", voidSchema),
  presencePreview: (settings, gameId, cycleIndex, stateCycleIndex) =>
    call("presence_preview", presenceActivitySchema, {
      settings,
      gameId,
      cycleIndex,
      stateCycleIndex,
    }),
  presenceHasToken: () => call("presence_has_token", z.boolean()),
  presenceSaveToken: (token) => call("presence_save_token", voidSchema, { token }),
  presenceClearToken: () => call("presence_clear_token", voidSchema),
  presenceTokenPreview: () => call("presence_token_preview", z.string().nullable()),
  presenceImageKeySave: (key) => call("presence_image_key_save", voidSchema, { key }),
  presenceImageKeyClear: () => call("presence_image_key_clear", voidSchema),
  presenceImageKeyPreview: () => call("presence_image_key_preview", z.string().nullable()),
  walkthroughGet: (gameId: string, vnId?: string | null) =>
    call("walkthrough_get", walkthroughViewSchema.nullable(), { gameId, vnId: vnId ?? null }),
  walkthroughSearchCandidate: (gameId: string, vnId?: string | null) =>
    call("walkthrough_search_candidate", walkthroughIndexEntrySchema.array(), { gameId, vnId: vnId ?? null }),
  walkthroughSearch: (query: string) =>
    call("walkthrough_search", walkthroughIndexEntrySchema.array(), { query }),
  walkthroughFetch: (gameId: string, vnId?: string | null, url?: string | null) =>
    call("walkthrough_fetch", walkthroughViewSchema, { gameId, vnId: vnId ?? null, url: url ?? null }),
  walkthroughTranslate: (
    gameId: string,
    provider: "ai" | "google",
    targetLang: "en" | "id",
    vnId?: string | null,
  ) =>
    call("walkthrough_translate", walkthroughViewSchema, {
      gameId,
      provider,
      targetLang,
      vnId: vnId ?? null,
    }),
  walkthroughToggleItem: (gameId: string, itemId: string) =>
    call("walkthrough_toggle_item", z.string().array(), { gameId, itemId }),
  walkthroughClearProgress: (gameId: string) =>
    call("walkthrough_clear_progress", voidSchema, { gameId }),
  openExternal: (url) => call("open_external", voidSchema, { url }),
  hideWindow: () => call("hide_window", voidSchema),
  minimizeWindow: () => call("minimize_window", voidSchema),
  showWindow: () => call("show_window", voidSchema),
  toggleWindow: () => call("toggle_window", z.boolean()),
  quitApp: () => call("quit_app", voidSchema),
};

/** True when running inside the Tauri shell. */
export const runningInTauri = isTauri;
