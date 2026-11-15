import { platform } from "@tauri-apps/plugin-os";

import { commands as annotate } from "@anlg/plugin-annotate";

import { isAppStoreBuild } from "~/shared/app-store";
import { useConfigValue } from "~/shared/config";

/** Annotate records on macOS and Linux, and not in the App Store build. */
export function annotateAvailable() {
  const os = platform();
  return (os === "macos" || os === "linux") && !isAppStoreBuild();
}

/**
 * Annotate on Linux: the desktop's sharing dialog picks the screen, there is
 * no permission to allow ahead, and the capsule and ink are in the video.
 */
export function annotateOnLinux() {
  return platform() === "linux";
}

export async function openScreenRecordingSettings() {
  const result = await annotate.openScreenRecordingSettings();
  if (result.status === "error") throw new Error(result.error);
}

/** Annotate's shortcuts while it is on, which dictation's must not reuse. */
export function useAnnotateShortcuts(): string[] {
  const enabled = useConfigValue("annotate_enabled");
  const shortcut = useConfigValue("annotate_shortcut");
  const paste = useConfigValue("annotate_paste_shortcut");
  return enabled && annotateAvailable() ? [shortcut, paste] : [];
}
