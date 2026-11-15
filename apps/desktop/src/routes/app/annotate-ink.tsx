import { createFileRoute } from "@tanstack/react-router";

import { AnnotateInkScreen } from "~/annotate/linux/ink";

export const Route = createFileRoute("/app/annotate-ink")({
  component: AnnotateInkScreen,
});
