import { i18n } from "@lingui/core";
import { I18nProvider } from "@lingui/react";
import {
  act,
  cleanup,
  render as renderInto,
  waitFor,
} from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  settings: {} as Record<string, unknown>,
  rows: [] as { id: string; raw_text: string; final_text: string }[],
  setTrayRecentDictations: vi.fn(),
  recentDictations: vi.fn(),
  writeText: vi.fn(),
  toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn() }),
  listener: null as ((event: { payload: { id: string } }) => void) | null,
}));
vi.mock("~/settings/queries", () => ({ useSettingsReady: () => true }));
vi.mock("~/shared/config", () => ({
  useConfigValue: (key: string) => mocks.settings[key],
}));
vi.mock("~/db", () => ({
  useLiveQuery: ({
    mapRows,
  }: {
    mapRows: (rows: typeof mocks.rows) => unknown;
  }) => ({ data: mapRows(mocks.rows) }),
}));
vi.mock("./history/store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./history/store")>()),
  recentDictations: mocks.recentDictations,
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: mocks.writeText,
}));
vi.mock("@anlg/plugin-tray", () => ({
  commands: { setTrayRecentDictations: mocks.setTrayRecentDictations },
  events: {
    copyRecentDictationEvent: {
      listen: async (
        listener: (event: { payload: { id: string } }) => void,
      ) => {
        mocks.listener = listener;
        return () => {
          if (mocks.listener === listener) mocks.listener = null;
        };
      },
    },
  },
}));
vi.mock("@anlg/ui/components/ui/toast", () => ({ toast: mocks.toast }));

import { dictationPreview, RecentDictationsTray } from "./recent-tray";

const render = (ui: ReactElement) =>
  renderInto(ui, {
    wrapper: ({ children }) => (
      <I18nProvider i18n={i18n}>{children}</I18nProvider>
    ),
  });

const lastMenu = () => mocks.setTrayRecentDictations.mock.lastCall?.[0];

describe("recent dictations in the tray", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.settings = {
      dictation_enabled: true,
      dictation_history_retention: "week",
    };
    mocks.rows = [];
    mocks.setTrayRecentDictations.mockResolvedValue({
      status: "ok",
      data: null,
    });
    mocks.writeText.mockResolvedValue(undefined);
  });

  afterEach(() => {
    cleanup();
  });

  it("previews a dictation on one line, cut with an ellipsis past 40 characters", () => {
    expect(dictationPreview("  Buy milk\n\tand  eggs  ")).toBe(
      "Buy milk and eggs",
    );
    const long = dictationPreview(
      "Remember to send the quarterly report to the whole team on Friday",
    );
    expect(long).toBe("Remember to send the quarterly report t…");
    expect(Array.from(long)).toHaveLength(40);
  });

  it("copies a chosen dictation's whole text, not its preview", async () => {
    const text =
      "Remember to send the quarterly report\nto the whole team on Friday.";
    mocks.rows = [{ id: "a1", raw_text: "um remember", final_text: text }];
    mocks.recentDictations.mockResolvedValue([{ id: "a1", text }]);
    render(<RecentDictationsTray />);
    await waitFor(() =>
      expect(lastMenu()).toMatchObject({
        items: [{ id: "a1", preview: dictationPreview(text) }],
      }),
    );

    await waitFor(() => expect(mocks.listener).not.toBeNull());
    await act(async () => {
      mocks.listener?.({ payload: { id: "a1" } });
    });

    await waitFor(() => expect(mocks.writeText).toHaveBeenCalledWith(text));
    expect(mocks.toast.success).toHaveBeenCalled();
  });

  it("lists no dictations while history is off, even with some stored", async () => {
    mocks.settings.dictation_history_retention = "off";
    mocks.rows = [{ id: "a1", raw_text: "", final_text: "Kept before." }];
    render(<RecentDictationsTray />);

    await waitFor(() =>
      expect(lastMenu()).toMatchObject({
        empty: "No recent dictations",
        items: [],
      }),
    );
  });

  it("replaces the menu when the history changes and clears it on unmount", async () => {
    mocks.rows = [{ id: "a1", raw_text: "", final_text: "First." }];
    const view = render(<RecentDictationsTray />);
    await waitFor(() =>
      expect(lastMenu()).toMatchObject({ items: [{ id: "a1" }] }),
    );

    mocks.rows = [
      { id: "b2", raw_text: "", final_text: "Second." },
      ...mocks.rows,
    ];
    view.rerender(<RecentDictationsTray />);
    await waitFor(() =>
      expect(lastMenu()).toMatchObject({
        items: [{ id: "b2" }, { id: "a1" }],
      }),
    );
    expect(
      mocks.setTrayRecentDictations.mock.calls.map(([menu]) => menu),
    ).not.toContain(null);

    view.unmount();
    await waitFor(() => expect(lastMenu()).toBeNull());
  });
});
