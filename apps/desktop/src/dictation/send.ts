import type { SendAfterInsert } from "@anlg/plugin-dictation";

import { getStoredSettingValues } from "~/settings/queries";
import { resolveConfigValues } from "~/shared/config";

/**
 * Where "Send after pasting" presses Return after a dictation, read as it is
 * inserted; null when it is off, or limited to apps and none are listed.
 */
export async function sendAfterInsert(): Promise<SendAfterInsert | null> {
  const {
    dictation_send_after_paste: mode,
    dictation_send_after_paste_apps: apps,
  } = resolveConfigValues(
    ["dictation_send_after_paste", "dictation_send_after_paste_apps"],
    await getStoredSettingValues(),
  );
  if (mode === "all") return { apps: [] };
  if (mode === "apps" && apps.length > 0) return { apps };
  return null;
}
