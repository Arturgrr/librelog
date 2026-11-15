import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  pressed: vi.fn(),
  hovered: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}));
vi.mock("@anlg/plugin-capture", () => ({
  commands: {
    cardStack: async () => ({
      status: "ok",
      data: {
        cards: [
          {
            id: "cap-2",
            kind: "video",
            thumbnailPath: "/thumbs/cap-2.png",
            saved: true,
            durationMs: 1500,
          },
          {
            id: "cap-1",
            kind: "screenshot",
            thumbnailPath: "/thumbs/cap-1.png",
            saved: false,
            durationMs: 0,
          },
        ],
        labels: {
          copy: "Copy",
          save: "Save",
          show: "Show in Files",
          edit: "Edit",
          close: "Close",
        },
      },
    }),
    cardPressed: async (...args: unknown[]) => {
      mocks.pressed(...args);
      return { status: "ok", data: true };
    },
    cardHovered: async (...args: unknown[]) => {
      mocks.hovered(...args);
      return { status: "ok", data: null };
    },
  },
  events: { captureCards: { listen: async () => () => {} } },
}));

import { CaptureCardsScreen } from "./cards";

afterEach(cleanup);

it("reports each card's buttons, its click and the pointer over it to the plugin", async () => {
  render(<CaptureCardsScreen />);
  const [video, shot] = await screen.findAllByRole("article");

  fireEvent.mouseEnter(shot);
  fireEvent.click(shot.querySelector("button")!);
  for (const name of ["Copy", "Save", "Edit", "Close"]) {
    fireEvent.click(
      Array.from(shot.querySelectorAll("button")).find(
        (button) => button.title === name,
      )!,
    );
  }
  fireEvent.mouseLeave(shot);
  expect(mocks.pressed.mock.calls).toEqual([
    ["cap-1", "edit"],
    ["cap-1", "copy"],
    ["cap-1", "save"],
    ["cap-1", "edit"],
    ["cap-1", "close"],
  ]);
  expect(mocks.hovered.mock.calls).toEqual([
    ["cap-1", true],
    ["cap-1", false],
  ]);

  // A saved recording opens on a click, shows its file, and has no Edit.
  mocks.pressed.mockClear();
  const titles = Array.from(video.querySelectorAll("button[title]")).map(
    (button) => button.getAttribute("title"),
  );
  expect(titles).toEqual(["Copy", "Show in Files", "Close"]);
  fireEvent.click(video.querySelector("button")!);
  expect(mocks.pressed.mock.calls).toEqual([["cap-2", "open"]]);
});
