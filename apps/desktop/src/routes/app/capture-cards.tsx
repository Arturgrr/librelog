import { createFileRoute } from "@tanstack/react-router";

import { CaptureCardsScreen } from "~/capture/linux/cards";

export const Route = createFileRoute("/app/capture-cards")({
  component: CaptureCardsScreen,
});
