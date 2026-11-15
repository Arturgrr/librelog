import { Trans } from "@lingui/react/macro";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { platform } from "@tauri-apps/plugin-os";
import { useEffect } from "react";

import {
  commands as shortcutCommands,
  type DesktopShortcutsEvent,
  events,
} from "@anlg/plugin-shortcut";
import { Button } from "@anlg/ui/components/ui/button";

import { shortcutKeys, useKeyLabel } from "./dictation-shortcut";

const QUERY_KEY = ["desktop-shortcuts"];

/** What the Wayland desktop did with Librelog's shortcuts. */
function useDesktopShortcuts() {
  const client = useQueryClient();
  const linux = platform() === "linux";
  useEffect(() => {
    if (!linux) return;
    const unlisten = events.desktopShortcutsEvent.listen(({ payload }) => {
      client.setQueryData<DesktopShortcutsEvent>(QUERY_KEY, payload);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [client, linux]);
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: () => shortcutCommands.desktopShortcuts(),
    enabled: linux,
    staleTime: Infinity,
  }).data;
}

/**
 * On Wayland the desktop approves Librelog's shortcuts and may bind other
 * keys: this says which keys it uses for `shortcuts`, and whether its prompt
 * is pending, was dismissed or failed (`failure` shows the last).
 */
export function DesktopShortcutsNote({
  shortcuts,
  failure = false,
}: {
  shortcuts: string[];
  failure?: boolean;
}) {
  const status = useDesktopShortcuts();
  const label = useKeyLabel();
  const ask = useMutation({
    mutationFn: () => shortcutCommands.askDesktopAgain(),
  });
  if (!status) return null;
  const mine = status.shortcuts.filter(
    (entry) => entry.shortcut !== null && shortcuts.includes(entry.shortcut),
  );
  const state = status.state;
  const askAgain = (
    <Button
      type="button"
      size="sm"
      variant="outline"
      className="self-start"
      disabled={ask.isPending}
      onClick={() => ask.mutate()}
    >
      <Trans>Ask my desktop again</Trans>
    </Button>
  );

  if (state.type === "dismissed" && mine.length > 0) {
    return (
      <div className="flex flex-col gap-2" role="status">
        <p className="text-muted-foreground text-xs">
          <Trans>
            Your desktop's shortcut prompt was dismissed, so these shortcuts
            don't work yet.
          </Trans>
        </p>
        {askAgain}
      </div>
    );
  }
  if (state.type === "failed" && failure) {
    return (
      <div className="flex flex-col gap-2" role="alert">
        <p className="text-destructive text-xs">{state.message}</p>
        {askAgain}
      </div>
    );
  }
  if (state.type === "waiting" && mine.length > 0) {
    return (
      <p className="text-muted-foreground text-xs" role="status">
        <Trans>Approve Librelog's shortcuts in your desktop's prompt.</Trans>
      </p>
    );
  }
  const differing = mine.filter((entry) => entry.differs && entry.trigger);
  if (state.type !== "bound" || differing.length === 0) return null;
  return (
    <ul className="text-muted-foreground flex flex-col gap-1 text-xs">
      {differing.map((entry) => {
        const keys = shortcutKeys(entry.shortcut ?? "")
          .map(label)
          .join(" ");
        const trigger = entry.trigger;
        return (
          <li key={entry.id}>
            <Trans>
              Your desktop uses {trigger} instead of {keys}.
            </Trans>
          </li>
        );
      })}
    </ul>
  );
}
