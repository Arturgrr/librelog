import { useLingui } from "@lingui/react/macro";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useState } from "react";

import {
  type CaptureCard,
  type CaptureCards,
  commands as capture,
  events as captureEvents,
} from "@anlg/plugin-capture";
import {
  Copy,
  DownloadSimple,
  FolderOpen,
  PencilSimple,
  Play,
  X,
} from "@anlg/ui/components/icons";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";
import { cn } from "@anlg/utils";

type Action = "copy" | "save" | "edit" | "open" | "close";

async function press(id: string, action: Action) {
  try {
    await capture.cardPressed(id, action);
  } catch (error) {
    console.error("[capture] card", error);
  }
}

async function hover(id: string, hovered: boolean) {
  try {
    await capture.cardHovered(id, hovered);
  } catch (error) {
    console.error("[capture] card", error);
  }
}

/**
 * The cards in the screen's corner on Linux, newest nearest the corner.
 * The plugin owns them; this window only shows them and reports presses.
 */
export function CaptureCardsScreen() {
  const [stack, setStack] = useState<CaptureCards | null>(null);

  useMountEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void captureEvents.captureCards
      .listen(({ payload }) => setStack(payload))
      .then((unlisten) => (disposed ? unlisten() : (stop = unlisten)));
    void (async () => {
      const result = await capture.cardStack();
      if (!disposed && result.status === "ok") {
        setStack((current) => current ?? result.data);
      }
    })();
    return () => {
      disposed = true;
      stop?.();
    };
  });

  return (
    <div className="flex h-screen w-screen flex-col-reverse gap-3 bg-transparent">
      {stack?.cards.map((card) => (
        <Card key={card.id} card={card} labels={stack.labels} />
      ))}
    </div>
  );
}

function Card({
  card,
  labels,
}: {
  card: CaptureCard;
  labels: CaptureCards["labels"];
}) {
  const { t } = useLingui();
  const screenshot = card.kind === "screenshot";
  const thumbnail = card.thumbnailPath
    ? convertFileSrc(card.thumbnailPath)
    : null;
  const buttons: { action: Action; label: string; icon: typeof Copy }[] = [
    { action: "copy", label: labels.copy, icon: Copy },
    card.saved
      ? { action: "save", label: labels.show, icon: FolderOpen }
      : { action: "save", label: labels.save, icon: DownloadSimple },
    ...(screenshot
      ? [{ action: "edit" as const, label: labels.edit, icon: PencilSimple }]
      : []),
    { action: "close", label: labels.close, icon: X },
  ];

  return (
    <article
      className={cn([
        "group relative h-[176px] w-full shrink-0 overflow-hidden rounded-xl",
        "bg-neutral-900 shadow-lg ring-1 ring-black/20",
      ])}
      onMouseEnter={() => void hover(card.id, true)}
      onMouseLeave={() => void hover(card.id, false)}
    >
      <button
        type="button"
        className="absolute inset-0 flex items-center justify-center"
        aria-label={screenshot ? labels.edit : t`Open`}
        onClick={() => void press(card.id, screenshot ? "edit" : "open")}
      >
        {thumbnail ? (
          <img
            src={thumbnail}
            alt=""
            draggable={false}
            className="h-full w-full object-contain"
          />
        ) : null}
        {!screenshot && (
          <Play className="absolute size-8 text-white/90 drop-shadow" />
        )}
      </button>
      <div
        className={cn([
          "absolute inset-x-0 bottom-0 flex justify-end gap-1 p-2",
          "bg-gradient-to-t from-black/60 to-transparent",
          "opacity-0 transition-opacity group-hover:opacity-100",
        ])}
      >
        {buttons.map(({ action, label, icon: Icon }) => (
          <button
            key={action}
            type="button"
            title={label}
            aria-label={label}
            className={cn([
              "flex size-8 items-center justify-center rounded-md",
              "bg-white/15 text-white hover:bg-white/30",
            ])}
            onClick={() => void press(card.id, action)}
          >
            <Icon className="size-4" />
          </button>
        ))}
      </div>
    </article>
  );
}
