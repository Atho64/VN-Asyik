import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import { PresencePanel } from "./PresencePanel";
import { api } from "@/lib/ipc/client";
import type { PresenceSettings } from "@/lib/ipc/types";

const emptyTransform = { prefix: "", suffix: "", remove: [], replace: [], trim: false };

const defaultSettings: PresenceSettings = {
  enabled: false,
  client_id: "",
  details_mode: "vndb",
  manual_details: "",
  manual_state: "",
  custom_details: "{vn_name}",
  custom_state: "{chapter}",
  custom_details_items: [],
  custom_state_items: [],
  custom_details_rotate: false,
  custom_state_rotate: false,
  custom_details_cycle_seconds: 5,
  custom_state_cycle_seconds: 5,
  vndb_details_items: [],
  vndb_state_template: "",
  vndb_swap_details_state: false,
  custom_details_transform: emptyTransform,
  custom_state_transform: emptyTransform,
  vndb_details_transform: emptyTransform,
  vndb_state_transform: emptyTransform,
  show_cover: true,
  show_buttons: true,
  show_list_button: false,
  my_vndb_link: "",
  show_timestamps: true,
  show_when_unfocused: true,
  activity_name: null,
  cycle_vndb_details: true,
  cycle_seconds: 5,
  vndb_loading_details: "Reading",
  vndb_loading_state: "Loading VNDB data...",
  rpc_image_host: "none",
};

vi.mock("@/lib/ipc/client", () => ({
  api: {
    presenceGetSettings: vi.fn(),
    presenceStatus: vi.fn(),
    presenceSetEnabled: vi.fn(),
    presenceSaveSettings: vi.fn(),
    presencePreview: vi.fn(),
    presenceHasToken: vi.fn(),
    presenceSaveToken: vi.fn(),
    presenceClearToken: vi.fn(),
    presenceTokenPreview: vi.fn(),
    presenceImageKeySave: vi.fn(),
    presenceImageKeyClear: vi.fn(),
    presenceImageKeyPreview: vi.fn(),
    listGames: vi.fn(),
  },
  // Mirrors the real class: the panel branches on `code` to decide whether the
  // message belongs under the field or only in a toast.
  IpcError: class IpcError extends Error {
    readonly code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [], platform: null });
  vi.mocked(api.presenceGetSettings).mockResolvedValue(defaultSettings);
  vi.mocked(api.presenceStatus).mockResolvedValue({ status: "disabled" });
  vi.mocked(api.presenceHasToken).mockResolvedValue(false);
  vi.mocked(api.presenceTokenPreview).mockResolvedValue(null);
  vi.mocked(api.presenceSaveToken).mockResolvedValue();
  vi.mocked(api.presenceClearToken).mockResolvedValue();
  vi.mocked(api.presenceImageKeySave).mockResolvedValue();
  vi.mocked(api.presenceImageKeyClear).mockResolvedValue();
  vi.mocked(api.presenceImageKeyPreview).mockResolvedValue(null);
  // The preview now names a real game, so the panel asks for the library.
  vi.mocked(api.listGames).mockResolvedValue([
    {
      game: {
        meta: {
          id: "g1",
          updated_at: 0,
          device_id: "d1",
          deleted: false,
        },
        title: "Steins;Gate",
        title_original: null,
        vndb_id: "v2002",
        status: "playing",
        rating: null,
        notes: null,
        length_minutes: null,
        manual_playtime_ms: 0,
        accent_color: null,
        cover_url: null,
        cover_sexual: null,
        cover_violence: null,
        custom_cover_path: null,
        custom_cover_source_vndb: false,
        started_at: null,
        finished_at: null,
      },
      tracked_ms: 0,
      afk_ms: 0,
      total_ms: 0,
    },
  ] as never);

  vi.mocked(api.presencePreview).mockResolvedValue({
    name: "Steins;Gate",
    details: "5pb. / Nitroplus",
    state: "Chapter 1",
    large_image: "https://t.vndb.org/cv/98/1098.jpg",
    large_text: "Steins;Gate",
    buttons: [{ label: "View on VNDB", url: "https://vndb.org/v2002" }],
  });
});

function renderPanel() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <PresencePanel />
    </AppContext.Provider>,
  );
}

describe("PresencePanel", () => {
  it("renders discord presence title and disabled status by default", async () => {
    renderPanel();

    expect(await screen.findByText("Discord Rich Presence")).toBeInTheDocument();
    expect(screen.getAllByText("Disabled").length).toBeGreaterThanOrEqual(1);
  });

  it("toggles enabled state and shows settings", async () => {
    const user = userEvent.setup();
    vi.mocked(api.presenceSetEnabled).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });

    renderPanel();
    const enableBtn = await screen.findByRole("button", { name: /disabled/i });
    await user.click(enableBtn);

    expect(api.presenceSetEnabled).toHaveBeenCalledWith(true);
    await waitFor(() => {
      expect(screen.getByText("Presence Mode")).toBeInTheDocument();
      expect(screen.getByText("Save presence settings")).toBeInTheDocument();
    });
  });

  it("titles the preview card with the VN, not the app name", async () => {
    // The regression this guards: the card heading was hardcoded to "VN Asyik",
    // which is exactly what Discord showed before activities carried a name.
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });

    renderPanel();

    // Scoped to the preview card: the scope picker and the preview-game picker
    // both list the same title, so a bare findByText would pass even if the card
    // itself were blank.
    // Wait for the title inside the card, not merely for the card: the shell
    // renders before the preview round-trip resolves, so waiting on the
    // container alone raced the async preview and failed only in a full run.
    const card = await waitFor(() => {
      const found = document.querySelector(".dc-card");
      expect(found).not.toBeNull();
      expect(within(found as HTMLElement).getByText("Steins;Gate")).toBeInTheDocument();
      return found as HTMLElement;
    });
    expect(within(card).getByText("Steins;Gate")).toBeInTheDocument();
    expect(screen.queryByText("VN Asyik")).not.toBeInTheDocument();
  });

  it("saves modified settings", async () => {
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      details_mode: "custom",
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });
    vi.mocked(api.presenceSaveSettings).mockResolvedValue(undefined);

    renderPanel();

    const saveBtn = await screen.findByText("Save presence settings");
    await user.click(saveBtn);

    expect(api.presenceSaveSettings).toHaveBeenCalled();
  });

  it("edits the custom rotation list and the preview steps through it", async () => {
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      details_mode: "custom",
      custom_details_items: ["entry one", "entry two"],
      custom_details_rotate: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });

    renderPanel();

    // Two entries means the preview can be stepped, which is the only way a user
    // can check entry 2 renders correctly without saving and launching a game.
    const next = await screen.findByRole("button", { name: "Next line" });
    await user.click(next);
    await waitFor(() => {
      // The game id is asserted, not `undefined`: passing no id was the bug that
      // made the preview always render the built-in sample.
      expect(api.presencePreview).toHaveBeenLastCalledWith(
        expect.objectContaining({ custom_details_items: ["entry one", "entry two"] }),
        "g1",
        1,
        0,
      );
    });
  });

  it("exposes the second VNDB List button only when it is switched on", async () => {
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_list_button: false,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });

    const { unmount } = renderPanel();
    expect(await screen.findByText("Show My VNDB List button")).toBeInTheDocument();
    expect(screen.queryByLabelText("Your VNDB profile")).not.toBeInTheDocument();
    unmount();

    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_list_button: true,
    });
    renderPanel();
    expect(await screen.findByLabelText("Your VNDB profile")).toBeInTheDocument();
  });

  it("offers the image host only while cover art shows, and the key field on ImgBB", async () => {
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_cover: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });

    const { unmount } = renderPanel();
    // Opt-in: with cover art on, the host chips exist but the key field waits
    // for imgbb specifically, and nothing is saved until the user asks.
    expect(await screen.findByText("Image Host")).toBeInTheDocument();
    expect(screen.queryByText("ImgBB API Key")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "ImgBB" }));
    expect(await screen.findByText("ImgBB API Key")).toBeInTheDocument();
    unmount();

    // The upload option only makes sense when the card can show art at all.
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_cover: false,
    });
    renderPanel();
    expect(await screen.findByText("Show Cover Art")).toBeInTheDocument();
    expect(screen.queryByText("Image Host")).not.toBeInTheDocument();
  });

  it("shows the server's reason when the profile link is rejected at save", async () => {
    // A rejected link that only logs a toast leaves the user adjusting the wrong
    // switch; the message has to sit on the field.
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_list_button: true,
      my_vndb_link: "https://example.com/u1",
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });
    const { IpcError } = await import("@/lib/ipc/client");
    vi.mocked(api.presenceSaveSettings).mockRejectedValue(
      new IpcError("invalid_profile_link", "must be a full https://vndb.org/u... URL"),
    );

    renderPanel();
    await user.click(await screen.findByText("Save presence settings"));

    expect(
      await screen.findByText("must be a full https://vndb.org/u... URL"),
    ).toBeInTheDocument();
  });

  it("renders two buttons on the preview card when both are configured", async () => {
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_list_button: true,
      my_vndb_link: "https://vndb.org/u123456",
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });
    // The preview now names a real game, so the panel asks for the library.
  vi.mocked(api.listGames).mockResolvedValue([
    {
      game: {
        meta: {
          id: "g1",
          updated_at: 0,
          device_id: "d1",
          deleted: false,
        },
        title: "Steins;Gate",
        title_original: null,
        vndb_id: "v2002",
        status: "playing",
        rating: null,
        notes: null,
        length_minutes: null,
        manual_playtime_ms: 0,
        accent_color: null,
        cover_url: null,
        cover_sexual: null,
        cover_violence: null,
        custom_cover_path: null,
        custom_cover_source_vndb: false,
        started_at: null,
        finished_at: null,
      },
      tracked_ms: 0,
      afk_ms: 0,
      total_ms: 0,
    },
  ] as never);

  vi.mocked(api.presencePreview).mockResolvedValue({
      name: "Steins;Gate",
      details: "5pb. / Nitroplus",
      state: "Reading",
      buttons: [
        { label: "View on VNDB", url: "https://vndb.org/v2002" },
        { label: "My VNDB List", url: "https://vndb.org/u123456" },
      ],
    });

    renderPanel();

    expect(await screen.findByText("View on VNDB")).toBeInTheDocument();
    expect(screen.getByText("My VNDB List")).toBeInTheDocument();
  });

  it("offers the background-card switch, on by default", async () => {
    // The setting exists because a reader alt-tabs constantly; clearing the card
    // on every switch made it blink out for most of a session.
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });
    vi.mocked(api.presenceSaveSettings).mockResolvedValue(undefined);

    renderPanel();

    expect(
      await screen.findByText("Keep showing when another window is focused"),
    ).toBeInTheDocument();

    await user.click(await screen.findByText("Save presence settings"));
    // The second argument is the scope: undefined means the global default.
    // Asserted so a save can never silently write a game's override instead.
    expect(api.presenceSaveSettings).toHaveBeenCalledWith(
      expect.objectContaining({ show_when_unfocused: true }),
      undefined,
    );
  });

  it("can switch the background card off", async () => {
    const user = userEvent.setup();
    vi.mocked(api.presenceGetSettings).mockResolvedValue({
      ...defaultSettings,
      enabled: true,
      show_when_unfocused: true,
    });
    vi.mocked(api.presenceStatus).mockResolvedValue({ status: "connected" });
    vi.mocked(api.presenceSaveSettings).mockResolvedValue(undefined);

    renderPanel();

    const row = (
      await screen.findByText("Keep showing when another window is focused")
    ).closest(".settings__row") as HTMLElement;
    await user.click(within(row).getByRole("button"));
    await user.click(screen.getByText("Save presence settings"));

    expect(api.presenceSaveSettings).toHaveBeenCalledWith(
      expect.objectContaining({ show_when_unfocused: false }),
      undefined,
    );
  });

  it("renders discord token row and ToS modal flow on android", async () => {
    const user = userEvent.setup();
    useAppStore.setState({
      platform: {
        os: "android",
        kind: "mobile",
        first_class: true,
        process_tracking: "experimental",
        chapter_detection: "unavailable",
        discord_presence: "supported",
        cloud_sync: "unavailable",
      },
    });

    renderPanel();

    expect(await screen.findByText("Discord User Token")).toBeInTheDocument();
    expect(screen.getByText("Token not configured")).toBeInTheDocument();

    const setTokenBtn = screen.getByRole("button", { name: "Set Discord Token" });
    await user.click(setTokenBtn);

    expect(screen.getByText("Discord Gateway & ToS Notice")).toBeInTheDocument();
    expect(
      screen.getByText("Self-Botting Disclaimer & Account Safety"),
    ).toBeInTheDocument();

    const acceptBtn = screen.getByRole("button", { name: "I Understand & Accept" });
    await user.click(acceptBtn);

    const input = screen.getByPlaceholderText("Paste your Discord user token here");
    expect(input).toBeInTheDocument();

    fireEvent.change(input, {
      target: {
        value: "mfa.W_test_discord_user_token_long_enough_to_pass_validation_1234567890",
      },
    });
    const saveTokenBtn = screen.getByRole("button", { name: "Save Discord Token" });
    await user.click(saveTokenBtn);

    expect(api.presenceSaveToken).toHaveBeenCalledWith(
      "mfa.W_test_discord_user_token_long_enough_to_pass_validation_1234567890",
    );
  });

  it("allows removing token when configured on android", async () => {
    const user = userEvent.setup();
    useAppStore.setState({
      platform: {
        os: "android",
        kind: "mobile",
        first_class: true,
        process_tracking: "experimental",
        chapter_detection: "unavailable",
        discord_presence: "supported",
        cloud_sync: "unavailable",
      },
    });
    vi.mocked(api.presenceHasToken).mockResolvedValue(true);
    vi.mocked(api.presenceTokenPreview).mockResolvedValue("mfa....7890");

    renderPanel();

    expect(await screen.findByText("mfa....7890")).toBeInTheDocument();
    const removeBtn = screen.getByRole("button", { name: "Remove Token" });
    await user.click(removeBtn);

    expect(api.presenceClearToken).toHaveBeenCalled();
  });
});
