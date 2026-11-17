import { BundleType, getBundleType } from "@tauri-apps/api/app";
import { platform } from "@tauri-apps/plugin-os";

let selfUpdate = true;

// Matches self_update_supported in plugins/updater2: on Linux only an AppImage
// replaces itself. RPM and deb installs, and unbundled builds (no bundle
// type), are told about a new version and sent to its release page.
export function supportsSelfUpdate(
  os: string,
  bundleType: BundleType | null,
): boolean {
  return os !== "linux" || bundleType === BundleType.AppImage;
}

export async function initializeSelfUpdate(): Promise<void> {
  const bundleType = await getBundleType().catch((error: unknown) => {
    console.error("[startup] failed to read the bundle type", error);
    return null;
  });
  selfUpdate = supportsSelfUpdate(platform(), bundleType ?? null);
}

export function canSelfUpdate(): boolean {
  return selfUpdate;
}
