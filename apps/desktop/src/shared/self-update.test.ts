import { BundleType } from "@tauri-apps/api/app";
import { describe, expect, it } from "vitest";

import { supportsSelfUpdate } from "./self-update";

describe("supportsSelfUpdate", () => {
  it("sends Linux RPM, deb and unbundled installs to the release page instead of installing", () => {
    expect(supportsSelfUpdate("linux", BundleType.AppImage)).toBe(true);
    expect(supportsSelfUpdate("linux", BundleType.Rpm)).toBe(false);
    expect(supportsSelfUpdate("linux", BundleType.Deb)).toBe(false);
    expect(supportsSelfUpdate("linux", null)).toBe(false);
    expect(supportsSelfUpdate("macos", BundleType.App)).toBe(true);
  });
});
