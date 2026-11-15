import { createFileRoute } from "@tanstack/react-router";

import { CaptureAreaScreen } from "~/capture/linux/area";

export const Route = createFileRoute("/app/capture-area")({
  component: CaptureAreaScreen,
});
