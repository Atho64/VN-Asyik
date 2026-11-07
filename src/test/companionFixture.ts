import { emptyVision } from "@/lib/ipc/companion";
import type {
  CompanionSession,
  CompanionSnapshot,
  ProfileCatalogue,
  ProfileSettings,
} from "@/lib/ipc/companion";
import type { ChatMessage } from "@/lib/ipc/chat";
import type { GameCard } from "@/lib/ipc/types";

export function companionProfiles(): ProfileCatalogue {
  const presets: ProfileSettings["preset"][] = [
    "companion",
    "kawaii",
    "analyst",
    "playful",
    "concise",
    "quiet_listener",
    "hype_friend",
    "mystery_partner",
    "romance_enthusiast",
    "cozy_book_club",
    "language_buddy",
    "wry_rival",
  ];
  return {
    presets: presets.map((preset, index) => ({
      id: `20b00000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
      version: 1,
      settings: {
        name: preset,
        preset,
        style: "",
        interests: "",
        length: "balanced",
        humor: 1,
        energy: 1,
        warmth: 2,
        questions: 1,
        language: "app",
      },
    })),
    profiles: [],
  };
}

export function companionSession(): CompanionSession {
  return {
    replayExcluded: false,
    id: "session-one",
    threadId: "chat-one",
    playthroughId: "reading-one",
    gameTitle: "Synthetic VN",
    options: {
      gameId: "game-one",
      playthroughLabel: "First reading",
      knowledge: "observed",
      boundary: "",
      source: "manual",
      language: "en",
      persona: "companion",
    },
    sourceUrl: null,
    state: "active",
    reason: null,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    endedAt: null,
  };
}

export function companionMessage(): ChatMessage {
  return {
    id: "answer-one",
    role: "assistant",
    content: "A quiet scene.",
    status: "running",
    error: null,
    tools: [],
    sources: [],
    tokens: null,
    createdAt: 1_700_000_000_000,
  };
}

export function companionSnapshot(sequence = 1): CompanionSnapshot {
  return {
    vision: { ...emptyVision },
    runtimeId: "runtime-one",
    requestId: null,
    sequence,
    epoch: 2,
    lifecycle: "active",
    session: companionSession(),
    busy: false,
    live: null,
    source: { state: "manual", lastLineAt: null, lineCount: 0, preview: [] },
    error: null,
    processId: null,
    capabilities: {
      overlay: "available",
      text: "available",
      luna: "available",
      backgroundRuntime: "available",
      processLifecycle: "available",
      vision: "unsupported",
      speech: "unsupported",
      durableMemory: "unsupported",
    },
  };
}

export function companionGame(): GameCard {
  return {
    game: {
      meta: { id: "game-one", updated_at: 1, device_id_device: "test", deleted: false },
      title: "Synthetic VN",
      title_original: null,
      vndb_id: null,
      status: "playing",
      rating: null,
      notes: null,
      cover_url: null,
      cover_sexual: null,
      cover_violence: null,
      manual_playtime_ms: 0,
      accent_color: null,
      custom_cover_path: null,
      length_minutes: null,
      custom_cover_source_vndb: false,
      started_at: null,
      finished_at: null,
    },
    tracked_ms: 0,
    afk_ms: 0,
    total_ms: 0,
  };
}
