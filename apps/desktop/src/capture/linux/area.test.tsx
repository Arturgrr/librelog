import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ chosen: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}));
vi.mock("@anlg/plugin-capture", () => ({
  commands: {
    areaFrame: async () => ({
      status: "ok",
      data: {
        imagePath: "/thumbs/area-frame-1.png",
        imageWidth: 1920,
        imageHeight: 1080,
        region: { x: 0, y: 0, width: 1920, height: 1080 },
        hint: "Drag to record",
      },
    }),
    areaChosen: async (choice: unknown) => {
      mocks.chosen(choice);
      return { status: "ok", data: null };
    },
  },
}));

import { CaptureAreaScreen } from "./area";

afterEach(cleanup);

it("answers from its on-screen buttons without the keyboard, once", async () => {
  render(<CaptureAreaScreen />);
  fireEvent.click(await screen.findByRole("button", { name: "Pick a window" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await vi.waitFor(() => expect(mocks.chosen).toHaveBeenCalled());
  expect(mocks.chosen.mock.calls).toEqual([[{ type: "window" }]]);
});
