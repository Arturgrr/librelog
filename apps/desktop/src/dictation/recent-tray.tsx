import { t } from "@lingui/core/macro";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useState } from "react";

import {
  commands as tray,
  events,
  type TrayRecentDictations,
} from "@anlg/plugin-tray";
import { toast } from "@anlg/ui/components/ui/toast";
import { useMountEffect } from "@anlg/ui/hooks/use-mount-effect";

import {
  type DictationHistoryRetention,
  mapRecentDictations,
  normalizeRetention,
  type RecentDictationRow,
  recentDictations,
  recentDictationsQuery,
  retentionCutoff,
} from "./history/store";

import { useLiveQuery } from "~/db";
import { localizeExternalMessage } from "~/i18n/external-messages";
import { useSettingsReady } from "~/settings/queries";
import { useConfigValue } from "~/shared/config";

const RECENT_LIMIT = 5;
const PREVIEW_LENGTH = 40;

let trayUpdates: Promise<void> = Promise.resolve();

/** One line of at most `PREVIEW_LENGTH` characters, ending in an ellipsis when cut. */
export function dictationPreview(text: string) {
  const line = text.replace(/\s+/gu, " ").trim();
  const characters = Array.from(line);
  if (characters.length <= PREVIEW_LENGTH) return line;
  return `${characters
    .slice(0, PREVIEW_LENGTH - 1)
    .join("")
    .trimEnd()}…`;
}

function unwrap<T>(
  result: { status: "ok"; data: T } | { status: "error"; error: string },
): T {
  if (result.status === "error") throw new Error(result.error);
  return result.data;
}

async function copyRecentDictation(id: string) {
  try {
    const entry = (await recentDictations(RECENT_LIMIT)).find(
      (recent) => recent.id === id,
    );
    if (!entry) {
      toast.error(t`This dictation is no longer in your history.`);
      return;
    }
    await writeText(entry.text);
    toast.success(t`Copied to clipboard`);
  } catch (error) {
    toast.error(
      localizeExternalMessage(
        error instanceof Error ? error.message : String(error),
      ),
    );
  }
}

function showTrayMenu(menu: TrayRecentDictations | null) {
  trayUpdates = trayUpdates
    .then(async () => {
      unwrap(await tray.setTrayRecentDictations(menu));
    })
    .catch((error) => console.error("[recent-dictations] tray menu", error));
}

/**
 * The tray's "Copy recent dictation" submenu, while dictation is on: the
 * newest dictations in the history, or none while history is off.
 */
export function RecentDictationsTray() {
  const settingsReady = useSettingsReady();
  const enabled = useConfigValue("dictation_enabled");
  const retention = normalizeRetention(
    useConfigValue("dictation_history_retention"),
  );
  if (!settingsReady || !enabled) return null;
  return (
    <>
      <CopyRequests />
      {retention === "off" ? (
        <RecentTrayMenu entries={[]} />
      ) : (
        <HistoryRecentDictations key={retention} retention={retention} />
      )}
    </>
  );
}

function CopyRequests() {
  useMountEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void events.copyRecentDictationEvent
      .listen(({ payload }) => void copyRecentDictation(payload.id))
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
  return null;
}

// Entries older than the cutoff are purged hourly, which updates the query.
function HistoryRecentDictations({
  retention,
}: {
  retention: DictationHistoryRetention;
}) {
  const [cutoff] = useState(() => retentionCutoff(retention));
  const { data } = useLiveQuery<
    RecentDictationRow,
    { id: string; text: string }[]
  >({
    ...recentDictationsQuery(cutoff, RECENT_LIMIT),
    mapRows: mapRecentDictations,
  });
  if (data === undefined) return null;
  return <RecentTrayMenu entries={data} />;
}

function RecentTrayMenu({
  entries,
}: {
  entries: { id: string; text: string }[];
}) {
  const menu: TrayRecentDictations = {
    title: t`Copy recent dictation`,
    empty: t`No recent dictations`,
    items: entries
      .filter((entry) => entry.text.trim())
      .map((entry) => ({
        id: entry.id,
        preview: dictationPreview(entry.text),
      })),
  };
  return <ShowTrayMenu key={JSON.stringify(menu)} menu={menu} />;
}

function ShowTrayMenu({ menu }: { menu: TrayRecentDictations }) {
  useMountEffect(() => {
    showTrayMenu(menu);
    return () => showTrayMenu(null);
  });
  return null;
}
