import { createFileRoute } from "@tanstack/react-router";

import { AnnotateCapsuleScreen } from "~/annotate/linux/capsule";

export const Route = createFileRoute("/app/annotate-capsule")({
  component: AnnotateCapsuleScreen,
});
