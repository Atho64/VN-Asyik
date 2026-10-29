import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { AppContext } from "@/app/AppProvider";
import { useAppStore } from "@/app/store";
import { createTranslator } from "@/i18n";
import type { LlmProfiles } from "@/lib/ipc/types";

import { ChatPanel } from "./ChatPanel";
import { api } from "@/lib/ipc/client";

vi.mock("@/lib/ipc/client", () => ({
  api: {
    aiLlmProfiles: vi.fn(),
    aiLlmSaveProfile: vi.fn(),
    aiLlmActivateProfile: vi.fn(),
    aiLlmDeleteProfile: vi.fn(),
    aiLlmClearKey: vi.fn(),
  },
  // Mirrors the real class: the panel translates `code` and only falls back to
  // the message, so the test has to carry both.
  IpcError: class IpcError extends Error {
    readonly code: string;
    constructor(code: string, message: string) {
      super(message);
      this.code = code;
    }
  },
}));

const gemini: LlmProfiles = {
  active: "p1",
  supported: true,
  profiles: [
    {
      id: "p1",
      name: "Gemini",
      provider: "openai",
      baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
      model: "gemini-2.5-flash",
      hasKey: true,
      keyHint: "AIz...9f2k",
    },
    {
      id: "p2",
      name: "Lokal",
      provider: "openai",
      baseUrl: "http://127.0.0.1:11434/v1",
      model: "qwen3",
      hasKey: false,
      keyHint: null,
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ toasts: [], platform: null });
  vi.mocked(api.aiLlmProfiles).mockResolvedValue(gemini);
});

function renderPanel() {
  return render(
    <AppContext.Provider
      value={{ t: createTranslator("en"), locale: "en", ready: true, bootError: null }}
    >
      <ChatPanel />
    </AppContext.Provider>,
  );
}

const row = (name: string) =>
  screen.getByText(name).closest("li") as HTMLElement;

describe("ChatPanel", () => {
  it("lists saved providers and marks the live one", async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText("Gemini")).toBeInTheDocument());
    expect(within(row("Gemini")).getByText("Active")).toBeInTheDocument();
    expect(within(row("Gemini")).getByText("Key stored")).toBeInTheDocument();
    // The endpoint travels with the model, so an endpoint edit is visible without
    // opening the form.
    expect(
      within(row("Gemini")).getByText("OpenAI-compatible · gemini-2.5-flash"),
    ).toBeInTheDocument();
    expect(
      within(row("Gemini")).getByText(
        "https://generativelanguage.googleapis.com/v1beta/openai",
      ),
    ).toBeInTheDocument();
    // Only the profile that is not active offers to be selected.
    expect(within(row("Gemini")).queryByRole("button", { name: "Use" })).toBeNull();
    expect(within(row("Lokal")).getByRole("button", { name: "Use" })).toBeInTheDocument();
  });

  it("saves a new provider with its key", async () => {
    vi.mocked(api.aiLlmSaveProfile).mockResolvedValue({
      ...gemini,
      profiles: [
        ...gemini.profiles,
        {
          id: "p3",
          name: "OpenAI",
          provider: "openai",
          baseUrl: "https://api.openai.com/v1",
          model: "gpt-4o-mini",
          hasKey: true,
          keyHint: "sk-...abcd",
        },
      ],
    });
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "OpenAI" } });
    fireEvent.change(screen.getByLabelText(/^Endpoint/), {
      target: { value: "https://api.openai.com/v1" },
    });
    fireEvent.change(screen.getByLabelText("Model"), {
      target: { value: "gpt-4o-mini" },
    });
    fireEvent.change(screen.getByLabelText(/^API key/), {
      target: { value: "sk-test-key-1234" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.aiLlmSaveProfile).toHaveBeenCalledWith({
        name: "OpenAI",
        provider: "openai",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        key: "sk-test-key-1234",
      }),
    );
    // The form closes on success, so the next click is not a second save.
    await waitFor(() => expect(screen.queryByLabelText("Name")).toBeNull());
  });

  it("starts a profile from a provider preset, Anthropic included", async () => {
    vi.mocked(api.aiLlmSaveProfile).mockResolvedValue(gemini);
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));

    // Anthropic is not OpenAI-shaped, so the preset has to pick the wire format
    // as well as the obvious fields.
    fireEvent.click(screen.getByRole("button", { name: "Anthropic" }));
    expect(screen.getByLabelText("Name")).toHaveValue("Anthropic");
    expect(screen.getByLabelText(/^Endpoint/)).toHaveValue(
      "https://api.anthropic.com/v1",
    );
    expect(screen.getByLabelText(/^Model/)).toHaveValue("claude-sonnet-4-5");

    fireEvent.change(screen.getByLabelText(/^API key/), {
      target: { value: "sk-ant-test-1234" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(api.aiLlmSaveProfile).toHaveBeenCalledWith(
        expect.objectContaining({
          provider: "anthropic",
          baseUrl: "https://api.anthropic.com/v1",
          model: "claude-sonnet-4-5",
        }),
      ),
    );
  });

  it("edits an existing provider without sending an empty key", async () => {
    vi.mocked(api.aiLlmSaveProfile).mockResolvedValue(gemini);
    renderPanel();
    await screen.findByText("Gemini");
    fireEvent.click(within(row("Gemini")).getByRole("button", { name: "Edit" }));

    expect(screen.getByLabelText("Name")).toHaveValue("Gemini");
    fireEvent.change(screen.getByLabelText("Model"), {
      target: { value: "gemini-2.5-pro" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.aiLlmSaveProfile).toHaveBeenCalledWith({
        id: "p1",
        name: "Gemini",
        provider: "openai",
        baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
        model: "gemini-2.5-pro",
      }),
    );
  });

  it("selects another provider", async () => {
    vi.mocked(api.aiLlmActivateProfile).mockResolvedValue({ ...gemini, active: "p2" });
    renderPanel();
    await screen.findByText("Lokal");
    fireEvent.click(within(row("Lokal")).getByRole("button", { name: "Use" }));
    await waitFor(() => expect(api.aiLlmActivateProfile).toHaveBeenCalledWith("p2"));
    await waitFor(() =>
      expect(within(row("Lokal")).getByText("Active")).toBeInTheDocument(),
    );
  });

  it("asks before deleting and then removes the provider", async () => {
    vi.mocked(api.aiLlmDeleteProfile).mockResolvedValue({
      ...gemini,
      profiles: [gemini.profiles[0]!],
    });
    renderPanel();
    await screen.findByText("Lokal");
    fireEvent.click(within(row("Lokal")).getByRole("button", { name: "Delete" }));
    // The first click only arms the control: deleting also deletes the key.
    expect(api.aiLlmDeleteProfile).not.toHaveBeenCalled();
    fireEvent.click(
      within(row("Lokal")).getByRole("button", { name: "Delete permanently" }),
    );
    await waitFor(() => expect(api.aiLlmDeleteProfile).toHaveBeenCalledWith("p2"));
    await waitFor(() => expect(screen.queryByText("Lokal")).toBeNull());
  });

  it("translates the backend's reason instead of showing its code", async () => {
    const { IpcError } = await import("@/lib/ipc/client");
    vi.mocked(api.aiLlmSaveProfile).mockRejectedValue(
      new IpcError("url_insecure", "the endpoint must use https"),
    );
    renderPanel();
    fireEvent.click(await screen.findByRole("button", { name: "Add provider" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Rumah" } });
    fireEvent.change(screen.getByLabelText(/^Endpoint/), {
      target: { value: "http://192.168.1.10:8000/v1" },
    });
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "m" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "use https, or http on localhost",
    );
    // A rejected save keeps the form open, so the value can be corrected.
    expect(screen.getByLabelText("Name")).toHaveValue("Rumah");
  });

  it("removes a stored key without deleting the provider", async () => {
    vi.mocked(api.aiLlmClearKey).mockResolvedValue({
      ...gemini,
      profiles: [{ ...gemini.profiles[0]!, hasKey: false, keyHint: null }, gemini.profiles[1]!],
    });
    renderPanel();
    await screen.findByText("Gemini");
    fireEvent.click(within(row("Gemini")).getByRole("button", { name: "Remove key" }));
    await waitFor(() => expect(api.aiLlmClearKey).toHaveBeenCalledWith("p1"));
    await waitFor(() =>
      expect(within(row("Gemini")).getByText("No key")).toBeInTheDocument(),
    );
  });
});
