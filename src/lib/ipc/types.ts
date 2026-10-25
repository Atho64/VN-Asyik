/**
 * Typed mirror of the Rust command surface.
 *
 * These types are hand-kept in sync with `src-tauri/src/db/models.rs` and
 * `src-tauri/src/commands`. Validity is enforced at runtime by the zod schemas
 * below, so a Rust change that breaks the contract fails loudly at the boundary
 * instead of corrupting the UI silently.
 */

import { z } from "zod";

export const androidAppSchema = z.object({ package: z.string(), label: z.string() });
export type AndroidApp = z.infer<typeof androidAppSchema>;
export const androidSessionViewSchema = z.object({
  active: z.boolean(),
  gameId: z.string().nullable(),
  gameTitle: z.string().nullable().optional(),
  package: z.string().nullable(),
  runnerKind: z.string().nullable().optional(),
  startedAtMs: z.number().nullable(),
  countedMs: z.number(),
  elapsedMs: z.number().optional(),
  lastImportedAtMs: z.number().nullable(),
  reason: z.string().nullable(),
});
export type AndroidSessionView = z.infer<typeof androidSessionViewSchema>;

export const gameStatusSchema = z.enum([
  "wishlist",
  "playing",
  "completed",
  "on_hold",
  "dropped",
]);
export type GameStatus = z.infer<typeof gameStatusSchema>;

export const sessionSourceSchema = z.enum([
  "desktop",
  "android_usage",
  "manual",
  "import",
]);
export type SessionSource = z.infer<typeof sessionSourceSchema>;

export const nsfwModeSchema = z.enum(["show", "blur", "hide"]);
export type NsfwMode = z.infer<typeof nsfwModeSchema>;

export const themeModeSchema = z.enum(["dark", "light", "system"]);
export type ThemeMode = z.infer<typeof themeModeSchema>;

export const syncMetaSchema = z.object({
  id: z.string(),
  updated_at: z.number(),
  device_id_device: z.string(),
  deleted: z.boolean(),
});
export type SyncMeta = z.infer<typeof syncMetaSchema>;

export const gameSchema = z.object({
  meta: syncMetaSchema,
  title: z.string(),
  title_original: z.string().nullable(),
  vndb_id: z.string().nullable(),
  status: gameStatusSchema,
  rating: z.number().nullable(),
  notes: z.string().nullable(),
  length_minutes: z.number().nullable(),
  manual_playtime_ms: z.number(),
  accent_color: z.string().nullable(),
  cover_url: z.string().nullable(),
  cover_sexual: z.number().nullable(),
  cover_violence: z.number().nullable(),
  /** App-data-relative path to a user-chosen cover, or null for none. */
  custom_cover_path: z.string().nullable(),
  /** True when the custom cover came from VNDB rather than the user's disk. */
  custom_cover_source_vndb: z.boolean(),
  started_at: z.number().nullable(),
  finished_at: z.number().nullable(),
});
export type Game = z.infer<typeof gameSchema>;

export const gameCardSchema = z.object({
  game: gameSchema,
  /** Active time only. The headline number everywhere in the UI. */
  tracked_ms: z.number(),
  /** Away time recorded separately, never folded into tracked_ms. */
  afk_ms: z.number(),
  total_ms: z.number(),
});
export type GameCard = z.infer<typeof gameCardSchema>;

export const chapterSchema = z.object({
  meta: syncMetaSchema,
  game_id: z.string(),
  name: z.string(),
  sort_index: z.number(),
  reached: z.boolean(),
  first_seen_at: z.number().nullable(),
});
export type Chapter = z.infer<typeof chapterSchema>;

export const playSessionSchema = z.object({
  meta: syncMetaSchema,
  game_id: z.string(),
  chapter_id: z.string().nullable(),
  started_at: z.number(),
  ended_at: z.number(),
  counted_ms: z.number(),
  afk_ms: z.number(),
  source: sessionSourceSchema,
});
export type PlaySession = z.infer<typeof playSessionSchema>;

export const endingSchema = z.object({
  meta: syncMetaSchema,
  game_id: z.string(),
  name: z.string(),
  route: z.string().nullable(),
  /* Linked route. Optional because an ending saved before routes existed has no
   * link, and a payload that predates the field must still decode. */
  route_id: z.string().nullable().optional(),
  achieved: z.boolean(),
  achieved_at: z.number().nullable(),
});
export type Ending = z.infer<typeof endingSchema>;

/**
 * What the add/edit form submits.
 *
 * `gameId` is only read when creating: an edit knows the ending's id, and the row
 * it belongs to must not move between games.
 *
 * `achievedAt` is the moment the route was finished. `null` means "now" on create
 * and "keep what is stored" on edit, which is what makes re-saving a name safe.
 */
export interface EndingInput {
  gameId: string;
  name: string;
  route: string | null;
  achieved: boolean;
  achievedAt: number | null;
}

export const statusBreakdownSchema = z.object({
  playing: z.number(),
  wishlist: z.number(),
  completed: z.number(),
  on_hold: z.number(),
  dropped: z.number(),
});
export type StatusBreakdown = z.infer<typeof statusBreakdownSchema>;

export const searchHitSchema = z.object({
  vn_id: z.string(),
  title: z.string(),
  alttitle: z.string().nullable(),
  released: z.string().nullable(),
  image_url: z.string().nullable(),
  sexual: z.number().nullable(),
  violence: z.number().nullable(),
  length_minutes: z.number().nullable(),
  length: z.number().nullable(),
  votecount: z.number().nullable(),
  in_library: z.boolean(),
});
export type SearchHit = z.infer<typeof searchHitSchema>;

export const searchResponseSchema = z.object({
  hits: searchHitSchema.array(),
  more: z.boolean(),
  from_cache: z.boolean(),
});
export type SearchResponse = z.infer<typeof searchResponseSchema>;

export const vndbTagSchema = z.object({
  id: z.string().nullable(),
  name: z.string().nullable(),
  category: z.string().nullable(),
  rating: z.number().nullable(),
  spoiler: z.number().nullable(),
  lie: z.boolean().nullable(),
});
export type VndbTag = z.infer<typeof vndbTagSchema>;

/** One title VNDB publishes for a VN, in one language and script. */
export const titleVariantSchema = z.object({
  lang: z.string().nullable(),
  title: z.string().nullable(),
  latin: z.string().nullable(),
  official: z.boolean().nullable(),
  main: z.boolean().nullable(),
});
export type TitleVariant = z.infer<typeof titleVariantSchema>;

/**
 * A VNDB image: the cover, a screenshot or a character portrait.
 *
 * Ratings arrive as hundredths of VNDB's 0..2 average (`sexual`, `violence`) so a
 * fractional vote survives the wire as an integer. `null` means VNDB does not know,
 * which is not the same as zero and must never be treated as consent.
 */
export const vndbImageSchema = z.object({
  id: z.string().nullable().default(null),
  url: z.string().nullable().default(null),
  dims: z.number().array().nullable().default(null),
  sexual: z.number().nullable().default(null),
  violence: z.number().nullable().default(null),
  votecount: z.number().nullable().default(null),
  thumbnail: z.string().nullable().default(null),
  thumbnail_dims: z.number().array().nullable().default(null),
});
export type VndbImage = z.infer<typeof vndbImageSchema>;

/** A developer or publisher, as VNDB lists it on the entry. */
export const vndbProducerSchema = z.object({
  id: z.string().nullable().default(null),
  name: z.string().nullable().default(null),
  original: z.string().nullable().default(null),
  type: z.string().nullable().default(null),
});
export type VndbProducer = z.infer<typeof vndbProducerSchema>;

/** A link from the VN's VNDB page to somewhere else: official site, wiki, shop. */
export const vndbExtLinkSchema = z.object({
  url: z.string().nullable().default(null),
  label: z.string().nullable().default(null),
  name: z.string().nullable().default(null),
});
export type VndbExtLink = z.infer<typeof vndbExtLinkSchema>;

/** One character on a VN's cast list. */
export const vndbCharacterSchema = z.object({
  id: z.string().nullable().default(null),
  name: z.string().nullable().default(null),
  original: z.string().nullable().default(null),
  /** `main`, `primary`, `side` or `appears`. */
  role: z.string().nullable().default(null),
  image: vndbImageSchema.nullable().default(null),
});
export type VndbCharacter = z.infer<typeof vndbCharacterSchema>;

export const detailPayloadSchema = z.object({
  vn_id: z.string(),
  title: z.string(),
  alttitle: z.string().nullable(),
  /** Original, romanised and English titles. Empty for rows cached before 0016. */
  titles: titleVariantSchema.array().default([]),
  /** Short names VNDB lists separately from titles. Empty before 0017. */
  aliases: z.string().array().default([]),
  length_minutes: z.number().nullable(),
  length: z.number().nullable(),
  /** How many people the average playtime is based on. Empty before 0018. */
  length_votes: z.number().nullable().default(null),
  description: z.string().nullable(),
  rating: z.number().nullable(),
  /** Raw vote average before the Bayesian correction. */
  average: z.number().nullable().default(null),
  votecount: z.number().nullable(),
  released: z.string().nullable(),
  olang: z.string().nullable(),
  /** 0 finished, 1 in development, 2 cancelled. */
  devstatus: z.number().nullable().default(null),
  languages: z.string().array().default([]),
  platforms: z.string().array().default([]),
  tags: vndbTagSchema.array(),
  screenshots: vndbImageSchema.array().default([]),
  developers: vndbProducerSchema.array().default([]),
  extlinks: vndbExtLinkSchema.array().default([]),
  characters: vndbCharacterSchema.array().default([]),
});
export type DetailPayload = z.infer<typeof detailPayloadSchema>;

/** One screenshot, resolved to paintable data URLs. */
export const screenshotViewSchema = z.object({
  image_id: z.string(),
  vndb_url: z.string(),
  /**
   * The sharp image, or the pre-blurred one when `sharp` is false. This is what a
   * revealed tile paints, exactly as `artUrl` is for a cover.
   */
  src: z.string(),
  /** The pre-blurred copy, when one is cached. */
  blurred_src: z.string().nullable().default(null),
  /** True when `src` is the sharp image and not the pre-blurred copy. */
  sharp: z.boolean(),
  /** True when VNDB's community rated this image explicit. */
  explicit: z.boolean(),
  width: z.number().nullable(),
  height: z.number().nullable(),
});
export type ScreenshotView = z.infer<typeof screenshotViewSchema>;

export const screenshotGallerySchema = z.object({
  shots: screenshotViewSchema.array(),
  /** How many VNDB reported, so a partial cache can say so. */
  reported: z.number(),
  /** False when some images could not be fetched. */
  complete: z.boolean(),
  /**
   * Screenshots still to download. Non-zero means "call again for more": the backend
   * downloads in batches so tiles appear progressively instead of after a full
   * minute of silence.
   */
  pending: z.number().default(0),
});
export type ScreenshotGallery = z.infer<typeof screenshotGallerySchema>;

export const importOutcomeSchema = z.object({
  game_id: z.string(),
  cover_cached: z.boolean(),
  cover_error: z.string().nullable(),
  already_owned: z.boolean(),
});
export type ImportOutcome = z.infer<typeof importOutcomeSchema>;

export const localArtSchema = z.object({
  cover_data_url: z.string().nullable(),
  backdrop_data_url: z.string().nullable(),
});
export type LocalArt = z.infer<typeof localArtSchema>;

/** Android usage-access state. `applicable` is false off Android. */
export const usageAccessViewSchema = z.object({
  applicable: z.boolean(),
  granted: z.boolean(),
  settingsAction: z.string().nullable(),
  reason: z.string().nullable(),
});

export type UsageAccessView = z.infer<typeof usageAccessViewSchema>;

/** Device-to-device sync status. */
/** Chat provider settings, without the key. */
export const chatSettingsSchema = z.object({
  baseUrl: z.string(),
  model: z.string(),
  hasKey: z.boolean(),
  keyHint: z.string().nullable(),
  supported: z.boolean(),
});
export type ChatSettings = z.infer<typeof chatSettingsSchema>;

/** One tag the library carries, with the games behind it. */
export const tasteThemeSchema = z.object({
  name: z.string(),
  count: z.number(),
  examples: z.array(z.string()),
  share: z.number(),
});

/** The library's taste: a plain count of its most common VNDB tags. */
export const tasteViewSchema = z.object({
  profile: z.object({
    considered: z.number(),
    undetermined: z.array(z.string()),
    themes: z.array(tasteThemeSchema),
  }),
});
export type TasteView = z.infer<typeof tasteViewSchema>;

export const syncStatusSchema = z.object({
  deviceId: z.string(),
  folder: z.string().nullable(),
  lastSyncAtMs: z.number().nullable(),
  games: z.number(),
  chapters: z.number(),
  sessions: z.number(),
});
export type SyncStatus = z.infer<typeof syncStatusSchema>;

/** A conflict that last-write-wins resolved. */
export const conflictViewSchema = z.object({
  table: z.string(),
  recordId: z.string(),
  kept: z.string(),
  reason: z.string(),
  discardedUpdatedAtMs: z.number(),
  createdAtMs: z.number(),
});
export type ConflictView = z.infer<typeof conflictViewSchema>;

/** What one sync run did. */
export const syncOutcomeSchema = z.object({
  peers: z.number(),
  pulled: z.number(),
  pushed: z.number(),
  conflicts: z.array(
    z.object({
      table: z.string(),
      recordId: z.string(),
      kept: z.string(),
      reason: z.string(),
      discardedUpdatedAtMs: z.number(),
    }),
  ),
  skipped: z.array(z.object({ file: z.string(), reason: z.string() })),
});
export type SyncOutcome = z.infer<typeof syncOutcomeSchema>;

export const trackerCapabilitySchema = z.object({
  available: z.boolean(),
  reason: z.string().nullable(),
  enabled: z.boolean(),
  poll_interval_ms: z.number(),
});
export type TrackerCapability = z.infer<typeof trackerCapabilitySchema>;

export const processCandidateSchema = z.object({
  pid: z.number(),
  exe_name: z.string(),
  game_id: z.string(),
  focused: z.boolean(),
});
export type ProcessCandidate = z.infer<typeof processCandidateSchema>;

export const detectCapabilitySchema = z.object({
  available: z.boolean(),
  reason: z.string().nullable(),
});
export type DetectCapability = z.infer<typeof detectCapabilitySchema>;

export const trackingStatusSchema = z.object({
  available: z.boolean(),
  unavailable_reason: z.string().nullable(),
  phase: z.string(),
  game_id: z.string().nullable(),
  exe_path: z.string().nullable(),
  pid: z.number().nullable(),
  active_ms: z.number(),
  afk_ms: z.number(),
  chapter_id: z.string().nullable(),
  candidates: processCandidateSchema.array(),
});
export type TrackingStatus = z.infer<typeof trackingStatusSchema>;

export const gameExecutableSchema = z.object({
  id: z.string(),
  game_id: z.string(),
  exe_name: z.string(),
  exe_path: z.string().nullable(),
  enabled: z.boolean(),
  runner_kind: z.string().default("native"),
  launch_target: z.string().nullable().optional(),
  title_match: z.string().nullable().optional(),
  extra_args: z.string().nullable().optional(),
});
export type GameExecutable = z.infer<typeof gameExecutableSchema>;

export type RunnerKind =
  | "vita3k"
  | "rpcs3"
  | "pcsx2"
  | "eden"
  | "citron"
  | "shadps4"
  | "ppsspp"
  | "winlator"
  | "gamehub"
  | "kirikiroid2"
  | "tyranor"
  | "onscripter"
  | "artemis"
  | "native_apk"
  | "custom";

export interface MapRunnerTargetInput {
  gameId: string;
  runnerKind: string;
  exePath?: string | null;
  package?: string | null;
  launchTarget?: string | null;
  titleMatch?: string | null;
  extraArgs?: string | null;
}

export const executableCandidateSchema = z.object({
  path: z.string(),
  exe_name: z.string(),
  size_bytes: z.number(),
  mapped_game_id: z.string().nullable(),
});
export type ExecutableCandidate = z.infer<typeof executableCandidateSchema>;

/* The Rust command serialises with `rename_all = "camelCase"`, which is the
 * project convention for view types. The schema must match the wire shape, not
 * the Rust field names. */
export const chapterViewSchema = z.object({
  id: z.string(),
  rawName: z.string(),
  displayName: z.string(),
  renamed: z.boolean(),
  /* Set by the user, not inferred from the name: a manual chapter whose name
   * happens to match its detected spelling is still manual. */
  manual: z.boolean(),
  order: z.number().nullable(),
  routeKey: z.string().nullable(),
  routeName: z.string().nullable(),
  /* Resolved by Rust: an engine route with an unrecorded name is labelled by its
   * code rather than falling back to the "no route" text. */
  routeLabel: z.string(),
  reached: z.boolean(),
  playtimeMs: z.number(),
  /* The route this chapter is linked to, when it has one. Distinct from
   * `routeKey`: that is the engine's detection key and changes with a game
   * build, while this is the user-visible route the Routes card groups on. */
  routeId: z.string().nullable(),
});
export type ChapterView = z.infer<typeof chapterViewSchema>;

/* One route with its chapters and endings nested, as `game_routes` returns it.
 *
 * Chapters and endings used to be two independent lists with two route
 * vocabularies, so one story appeared twice and neither half could show the
 * other. This is the joined shape that makes them one thing. */
export const routeViewSchema = z.object({
  /* Null for the bucket of rows that belong to no route at all. Those still have
   * to be listed, or creating a route would hide the chapters that prompted it. */
  id: z.string().nullable(),
  name: z.string(),
  /* `engine`, `ending` or `manual` -- where the row came from. Shown so a user can
   * tell why one story appears twice before merging the halves. */
  origin: z.string(),
  chapters: chapterViewSchema.array(),
  endings: endingSchema.array(),
  chaptersReached: z.number(),
  chaptersTotal: z.number(),
  endingsReached: z.number(),
  endingsTotal: z.number(),
  playtimeMs: z.number(),
  mergeCandidates: z.string().array(),
});
export type RouteView = z.infer<typeof routeViewSchema>;

/* What chapter detection can do here. `automatic` decides whether the panel
 * promises detection or points at the manual timer instead.
 *
 * `engine` and `unavailableReason` exist so the panel can say something useful
 * instead of the same sentence for every game: "no engine profile for this build"
 * and "map this game's executable first" are different problems with different
 * fixes. */
export const chapterCapabilityViewSchema = z.object({
  platform: z.string(),
  sources: z.array(z.string()),
  automatic: z.boolean(),
  engine: z.string().nullable(),
  unavailableReason: z.string().nullable(),
});

export type ChapterCapabilityView = z.infer<typeof chapterCapabilityViewSchema>;

export const manualTimerViewSchema = z.object({
  active: z.boolean(),
  chapterId: z.string().nullable(),
  /* Set instead of `chapterId` when the user timed a route with no chapter. */
  routeId: z.string().nullable(),
  chapterName: z.string().nullable(),
  elapsedMs: z.number(),
  running: z.boolean(),
});
export type ManualTimerView = z.infer<typeof manualTimerViewSchema>;

export const vndbConflictSchema = z.object({
  field: z.string(),
  local: z.string().nullable(),
  remote: z.string().nullable(),
});
export type VndbConflict = z.infer<typeof vndbConflictSchema>;

export const vndbAccountSchema = z.object({
  user_id: z.string(),
  username: z.string(),
  permissions: z.string().array(),
  connected_at: z.number(),
  last_pull_at: z.number().nullable(),
  last_push_at: z.number().nullable(),
});
export type VndbAccount = z.infer<typeof vndbAccountSchema>;

export const vndbStatusSchema = z.object({
  storage_supported: z.boolean(),
  account: vndbAccountSchema.nullable(),
  pending: z.number(),
});
export type VndbStatus = z.infer<typeof vndbStatusSchema>;

export const labelMappingSchema = z.object({
  playing: z.number(),
  completed: z.number(),
  on_hold: z.number(),
  dropped: z.number(),
  wishlist: z.number(),
});
export type LabelMapping = z.infer<typeof labelMappingSchema>;

export const conflictPolicySchema = z.enum([
  "ask",
  "prefer_local",
  "prefer_remote",
  "newest",
]);
export type ConflictPolicy = z.infer<typeof conflictPolicySchema>;

export const syncSettingsSchema = z.object({
  mapping: labelMappingSchema,
  policy: conflictPolicySchema,
});
export type SyncSettings = z.infer<typeof syncSettingsSchema>;

export const syncRowSchema = z.object({
  vn_id: z.string(),
  game_id: z.string().nullable(),
  title: z.string(),
  conflicts: vndbConflictSchema.array(),
});
export type SyncRow = z.infer<typeof syncRowSchema>;

/** One VNDB list entry to import, with the user's own state. */
export const missingEntrySchema = z.object({
  vnId: z.string(),
  status: z.string().nullable(),
  rating: z.number().nullable(),
  notes: z.string().nullable(),
  startedAt: z.number().nullable(),
  finishedAt: z.number().nullable(),
});
export type MissingEntry = z.infer<typeof missingEntrySchema>;

export const pullResultSchema = z.object({
  entries: z.number(),
  // Each entry carries the state the user already has on VNDB, so importing it
  // does not create a row that the next pull reports as a conflict.
  not_imported: missingEntrySchema.array(),
  conflicts: syncRowSchema.array(),
  in_sync: z.number(),
});
export type PullResult = z.infer<typeof pullResultSchema>;

export const applyResultSchema = z.object({
  updated: z.number(),
  queued: z.string().array(),
  remaining: z.number(),
});
export type ApplyResult = z.infer<typeof applyResultSchema>;

export const pushResultSchema = z.object({
  pushed: z.number(),
  failed: z.number(),
  remaining: z.number(),
});
export type PushResult = z.infer<typeof pushResultSchema>;

export const queueEntrySchema = z.object({
  vn_id: z.string(),
  game_id: z.string(),
  created_at: z.number(),
  attempts: z.number(),
  last_error: z.string().nullable(),
});
export type QueueEntry = z.infer<typeof queueEntrySchema>;

export const logEntrySchema = z.object({
  id: z.number(),
  at: z.number(),
  kind: z.string(),
  vn_id: z.string().nullable(),
  message: z.string(),
});
export type LogEntry = z.infer<typeof logEntrySchema>;

export const ulistLabelInfoSchema = z.object({
  id: z.number(),
  private: z.boolean().nullable(),
  label: z.string().nullable(),
  count: z.number().nullable(),
});
export type UlistLabelInfo = z.infer<typeof ulistLabelInfoSchema>;

export const verificationSchema = z.enum(["keep_local", "keep_remote", "skip"]);
export type VndbResolution = z.infer<typeof verificationSchema>;

export const afkConfigSchema = z.object({
  threshold_ms: z.number(),
  exclude_afk: z.boolean(),
});
export type AfkConfig = z.infer<typeof afkConfigSchema>;

export const prefsSchema = z.object({
  theme: z.string(),
  language: z.string(),
  nsfw: z.string(),
  reduce_effects: z.boolean(),
});
export type Prefs = z.infer<typeof prefsSchema>;

export const supportSchema = z.enum(["supported", "experimental", "unavailable"]);
export type Support = z.infer<typeof supportSchema>;

export const platformInfoSchema = z.object({
  os: z.string(),
  kind: z.string(),
  first_class: z.boolean(),
  process_tracking: supportSchema,
  chapter_detection: supportSchema,
  discord_presence: supportSchema,
  cloud_sync: supportSchema,
});
export type PlatformInfo = z.infer<typeof platformInfoSchema>;

export const importSummarySchema = z.object({
  games: z.number(),
  chapters: z.number(),
  sessions: z.number(),
  endings: z.number(),
  sessions_skipped: z.number(),
});
export type ImportSummary = z.infer<typeof importSummarySchema>;

/** A native file remains in Rust until this short-lived preview is confirmed. */
export const dataImportPreviewSchema = z.object({
  id: z.string(),
  kind: z.enum(["library", "backup", "transfer"]),
  createdAtMs: z.number(),
  games: z.number().int().nonnegative(),
  chapters: z.number().int().nonnegative(),
  sessions: z.number().int().nonnegative(),
  endings: z.number().int().nonnegative(),
  sameDevice: z.boolean(),
  canRestore: z.boolean(),
});
export type DataImportPreview = z.infer<typeof dataImportPreviewSchema>;

export const dataImportResultSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("backup"), summary: importSummarySchema }),
  z.object({ kind: z.literal("transfer"), outcome: syncOutcomeSchema }),
]);
export type DataImportResult = z.infer<typeof dataImportResultSchema>;

/** Error shape returned by every command failure. */
export interface CommandError {
  code: string;
  message: string;
}

/** What the recommendation screen can do. */
export const recommendSettingsSchema = z.object({
  hasTavilyKey: z.boolean(),
  tavilyKeyHint: z.string().nullable(),
  hasChatKey: z.boolean(),
  supported: z.boolean(),
  chatModel: z.string(),
});
export type RecommendSettings = z.infer<typeof recommendSettingsSchema>;

/** Only http(s) ever reaches `open_external`: these URLs come from a search API. */
const externalUrlSchema = z
  .string()
  .refine((value) => /^https?:\/\//i.test(value), "not an http(s) url");

/** A checkable link behind a suggestion. */
export const citationSchema = z.object({
  title: z.string(),
  url: externalUrlSchema,
});
export type Citation = z.infer<typeof citationSchema>;

/** One suggested game. */
export const recommendationSchema = z.object({
  title: z.string(),
  why: z.string(),
  sources: z.array(citationSchema),
  owned: z.boolean(),
});
export type Recommendation = z.infer<typeof recommendationSchema>;

/** A page the search returned. */
export const searchResultSchema = z.object({
  title: z.string(),
  url: externalUrlSchema,
  content: z.string(),
  relevance: z.number(),
});
export type SearchResult = z.infer<typeof searchResultSchema>;

/** The whole answer for one request. */
export const recommendOutcomeSchema = z.object({
  query: z.string(),
  games: z.array(recommendationSchema),
  answer: z.string().nullable(),
  sources: z.array(searchResultSchema),
  note: z.string().nullable(),
  credits: z.number(),
});
export type RecommendOutcome = z.infer<typeof recommendOutcomeSchema>;

/** One selectable Locale Emulator profile. */
export const launchProfileSchema = z.object({
  name: z.string(),
  guid: z.string(),
  location: z.string(),
  selected: z.boolean(),
});
export type LaunchProfile = z.infer<typeof launchProfileSchema>;

/** What the launch screen can do. */
/**
 * What Locale Emulator will do for the next launch.
 *
 * One value rather than several booleans, because only one combination is ever
 * truthful. `ready` is the only state that starts a game in Japanese; keying a
 * badge off "the launcher exists on disk" is what previously promised
 * localization that the launch then skipped.
 */
export const emulatorStateSchema = z.enum([
  "notInstalled",
  "noProfile",
  "ready",
  "disabled",
]);
export type EmulatorState = z.infer<typeof emulatorStateSchema>;

export const launchCapabilitySchema = z.object({
  canLaunch: z.boolean(),
  emulator: emulatorStateSchema,
  emulatorPath: z.string().nullable(),
  configuredDir: z.string().nullable(),
  guid: z.string().nullable(),
  profiles: z.array(launchProfileSchema),
  note: z.string().nullable(),
});
export type LaunchCapability = z.infer<typeof launchCapabilitySchema>;

/** Result of starting a game. */
export const launchOutcomeSchema = z.object({
  started: z.boolean(),
  localized: z.boolean(),
  command: z.string(),
  explanation: z.string(),
});
export type LaunchOutcome = z.infer<typeof launchOutcomeSchema>;

/** What a bulk metadata refresh did. */
export const refreshAllSchema = z.object({
  attempted: z.number(),
  lastIndex: z.number(),
  updated: z.number(),
  cancelled: z.boolean(),
  failed: z.string().array(),
});
export type RefreshAllOutcome = z.infer<typeof refreshAllSchema>;

/** What a bulk VNDB import did. */
export const bulkImportSchema = z.object({
  requested: z.number(),
  imported: z.number(),
  refreshed: z.number(),
  failed: z.string().array(),
  stoppedEarly: z.boolean(),
});
export type BulkImportOutcome = z.infer<typeof bulkImportSchema>;

/** Window-title parsing configuration. */
export const titleConfigSchema = z.object({
  format: z.string(),
  pattern: z.string().nullable(),
  sample: z.string().nullable(),
});
export type TitleConfig = z.infer<typeof titleConfigSchema>;

/** One tested caption. */
export const patternTestSchema = z.object({
  title: z.string(),
  matched: z.boolean(),
  route: z.string().nullable(),
  position: z.string().nullable(),
});
export type PatternTest = z.infer<typeof patternTestSchema>;

/** One bar in the time series. */
export const timeBucketSchema = z.object({
  startMs: z.number(),
  endMs: z.number(),
  playtimeMs: z.number(),
  sessions: z.number(),
});
export type TimeBucket = z.infer<typeof timeBucketSchema>;

/** Derived library statistics. */
export const derivedStatsSchema = z.object({
  completionRate: z.number().nullable(),
  averageRating: z.number().nullable(),
  longestSessionMs: z.number().nullable(),
  averageSessionMs: z.number().nullable(),
  sessionCount: z.number(),
  favouriteRoute: z.object({ name: z.string(), playtimeMs: z.number() }).nullable(),
  ratingSpread: z.number().array(),
  finishedThisYear: z.number().nullable(),
  longestStreakDays: z.number().nullable(),
});
export type DerivedStats = z.infer<typeof derivedStatsSchema>;

/** Statistics plus the series for the chosen period. */
export const statsPayloadSchema = z.object({
  summary: derivedStatsSchema,
  series: timeBucketSchema.array(),
});
export type StatsPayload = z.infer<typeof statsPayloadSchema>;

export const presenceDetailsModeSchema = z.enum(["vndb", "custom", "manual"]);
export type PresenceDetailsMode = z.infer<typeof presenceDetailsModeSchema>;

/** Where a cover Discord cannot fetch directly may be uploaded, so the card can
 * still show it. `none` keeps every image off the network. */
export const rpcImageHostSchema = z.enum(["none", "catbox", "imgbb"]);
export type RpcImageHost = z.infer<typeof rpcImageHostSchema>;

/** One `old` → `new` literal replacement in a field transform. */
export const replaceRuleSchema = z.object({
  old: z.string(),
  new: z.string(),
});
export type ReplaceRule = z.infer<typeof replaceRuleSchema>;

/** Per-field post-processing applied after template substitution. */
export const textTransformSchema = z.object({
  prefix: z.string(),
  suffix: z.string(),
  remove: z.string().array(),
  replace: replaceRuleSchema.array(),
  trim: z.boolean(),
});
export type TextTransform = z.infer<typeof textTransformSchema>;

export const presenceSettingsSchema = z.object({
  enabled: z.boolean(),
  client_id: z.string(),
  details_mode: presenceDetailsModeSchema,
  manual_details: z.string(),
  manual_state: z.string(),
  custom_details: z.string(),
  custom_state: z.string(),
  /** Templates the details line rotates through; empty falls back to the single template. */
  custom_details_items: z.string().array(),
  custom_state_items: z.string().array(),
  custom_details_rotate: z.boolean(),
  custom_state_rotate: z.boolean(),
  custom_details_cycle_seconds: z.number(),
  custom_state_cycle_seconds: z.number(),
  /** Templates the VNDB details line rotates through; empty uses the built-in facts. */
  vndb_details_items: z.string().array(),
  /** Empty keeps the built-in route/chapter/status state line. */
  vndb_state_template: z.string(),
  vndb_swap_details_state: z.boolean(),
  custom_details_transform: textTransformSchema,
  custom_state_transform: textTransformSchema,
  vndb_details_transform: textTransformSchema,
  vndb_state_transform: textTransformSchema,
  show_cover: z.boolean(),
  show_buttons: z.boolean(),
  show_list_button: z.boolean(),
  /** The user's own VNDB profile page, for the second button. */
  my_vndb_link: z.string(),
  show_timestamps: z.boolean(),
  /** Keep the card on Discord while the game runs but another window has focus. */
  show_when_unfocused: z.boolean(),
  activity_name: z.string().nullable().optional(),
  cycle_vndb_details: z.boolean(),
  cycle_seconds: z.number(),
  vndb_loading_details: z.string(),
  vndb_loading_state: z.string(),
  rpc_image_host: rpcImageHostSchema,
});
export type PresenceSettings = z.infer<typeof presenceSettingsSchema>;

export const presenceStatusSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("disabled") }),
  z.object({ status: z.literal("disconnected") }),
  z.object({ status: z.literal("connecting") }),
  z.object({ status: z.literal("connected") }),
  z.object({ status: z.literal("discord_not_running") }),
  z.object({ status: z.literal("error"), message: z.string() }),
  z.object({ status: z.literal("unsupported_platform") }),
]);
export type PresenceStatus = z.infer<typeof presenceStatusSchema>;

export const presenceButtonSchema = z.object({
  label: z.string(),
  url: z.string(),
});
export type PresenceButton = z.infer<typeof presenceButtonSchema>;

export const presenceActivitySchema = z.object({
  /** The card title Discord renders, above the details line. */
  name: z.string().nullable().optional(),
  details: z.string().nullable().optional(),
  state: z.string().nullable().optional(),
  large_image: z.string().nullable().optional(),
  large_text: z.string().nullable().optional(),
  small_image: z.string().nullable().optional(),
  small_text: z.string().nullable().optional(),
  start_timestamp: z.number().nullable().optional(),
  buttons: presenceButtonSchema.array(),
});
export type PresenceActivity = z.infer<typeof presenceActivitySchema>;
