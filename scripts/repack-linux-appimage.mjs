// Repacks the AppImage Tauri builds on Ubuntu so that it runs on Fedora, then
// re-signs it for the updater. Runs after `tauri build --bundles appimage`.

import { spawn } from "node:child_process";
import { constants } from "node:fs";
import {
  access,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Libraries that must come from the host rather than the Ubuntu build box:
// Wayland, because host Mesa and WebKitGTK need the matching ABI; PipeWire and
// its SPA modules, because libpipewire loads plugins from a directory baked in
// at build time (Ubuntu's multiarch path does not exist on Fedora), so a bundled
// copy silently fails to capture there. libpulse stays bundled: it only speaks a
// stable socket protocol to the daemon, and it dlopens its own libpulsecommon.
const hostLibraryPatterns = [
  /^libwayland-.*\.so(?:\..*)?$/,
  /^libpipewire-.*\.so(?:\..*)?$/,
  /^libspa-.*\.so(?:\..*)?$/,
];

// linuxdeploy's GTK hook forces X11, which matches the app running through
// XWayland on Wayland sessions; the patch keeps that default but lets a
// GDK_BACKEND the user exported through to the app.
const gdkBackendDefault = 'export GDK_BACKEND="${GDK_BACKEND:-x11}"';

export function patchGtkHook(source) {
  if (source.includes(gdkBackendDefault)) {
    return source;
  }
  const forcedX11 = /^export GDK_BACKEND=x11(?:[ \t]+#.*)?$/m;
  if (forcedX11.test(source)) {
    return source.replace(forcedX11, gdkBackendDefault);
  }
  if (/GDK_BACKEND/.test(source)) {
    throw new Error("Unrecognized GDK_BACKEND setting in AppImage GTK hook");
  }
  return source;
}

async function findSingleBundleEntry(bundleDirectory, suffix, isExpectedType) {
  const entries = await readdir(bundleDirectory, { withFileTypes: true });
  const matches = entries.filter(
    (entry) => entry.name.endsWith(suffix) && isExpectedType(entry),
  );

  if (matches.length !== 1) {
    throw new Error(
      `Expected exactly one ${suffix} in ${bundleDirectory}, found ${matches.length}`,
    );
  }

  return path.join(bundleDirectory, matches[0].name);
}

async function findBundledHostLibraries(directory) {
  const matches = [];

  async function walk(currentDirectory) {
    const entries = await readdir(currentDirectory, { withFileTypes: true });
    for (const entry of entries) {
      const entryPath = path.join(currentDirectory, entry.name);
      if (entry.isDirectory()) {
        await walk(entryPath);
      } else if (
        hostLibraryPatterns.some((pattern) => pattern.test(entry.name))
      ) {
        matches.push(entryPath);
      }
    }
  }

  await walk(directory);
  return matches.sort();
}

function defaultPluginPath() {
  const cacheDirectory =
    process.env.XDG_CACHE_HOME ?? path.join(os.homedir(), ".cache");
  return path.join(
    cacheDirectory,
    "tauri",
    "linuxdeploy-plugin-appimage.AppImage",
  );
}

function runCommand(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      env: options.env ?? process.env,
      stdio: "inherit",
    });

    child.on("error", reject);
    child.on("close", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }

      reject(
        new Error(
          `${command} exited with ${signal ? `signal ${signal}` : `code ${code}`}`,
        ),
      );
    });
  });
}

// Without the updater signing key (pull request builds) the stale signature
// is removed instead of shipped next to a repacked file it no longer matches.
export async function repackLinuxAppImage({
  bundleDirectory,
  arch = process.env.ARCH,
  pluginPath = process.env.TAURI_APPIMAGE_PLUGIN ?? defaultPluginPath(),
  sign = Boolean(process.env.TAURI_SIGNING_PRIVATE_KEY),
  run = runCommand,
}) {
  const appDirectory = await findSingleBundleEntry(
    bundleDirectory,
    ".AppDir",
    (entry) => entry.isDirectory(),
  );
  const appImage = await findSingleBundleEntry(
    bundleDirectory,
    ".AppImage",
    (entry) => entry.isFile(),
  );
  const hostLibraries = await findBundledHostLibraries(
    path.join(appDirectory, "usr"),
  );
  const gtkHook = path.join(
    appDirectory,
    "apprun-hooks",
    "linuxdeploy-plugin-gtk.sh",
  );
  const originalGtkHook = await readFile(gtkHook, "utf8");
  const patchedGtkHook = patchGtkHook(originalGtkHook);
  const updatedGtkHook = originalGtkHook !== patchedGtkHook;

  if (hostLibraries.length === 0 && !updatedGtkHook) {
    console.log(
      `AppImage already uses the host's Wayland and PipeWire libraries and GDK backend: ${appImage}`,
    );
    return { appImage, removedLibraries: [], updatedGtkHook, signed: false };
  }

  if (!arch) {
    throw new Error("AppImage architecture is required for repacking");
  }

  try {
    await access(pluginPath, constants.X_OK);
  } catch {
    throw new Error(`AppImage output plugin is not executable: ${pluginPath}`);
  }

  if (updatedGtkHook) {
    await writeFile(gtkHook, patchedGtkHook);
  }
  await Promise.all(hostLibraries.map((library) => rm(library)));

  const remainingLibraries = await findBundledHostLibraries(
    path.join(appDirectory, "usr"),
  );
  if (remainingLibraries.length > 0) {
    throw new Error(
      `Failed to remove bundled host libraries: ${remainingLibraries.join(", ")}`,
    );
  }

  console.log(`Removed bundled host libraries:\n${hostLibraries.join("\n")}`);

  await rm(appImage);
  await run(
    pluginPath,
    ["--appimage-extract-and-run", "--appdir", appDirectory],
    {
      env: {
        ...process.env,
        APPIMAGE_EXTRACT_AND_RUN: "1",
        ARCH: arch,
        OUTPUT: appImage,
      },
    },
  );

  const appImageStat = await stat(appImage);
  if (appImageStat.size === 0) {
    throw new Error(`Repacked AppImage is empty: ${appImage}`);
  }

  const signature = `${appImage}.sig`;
  await rm(signature, { force: true });
  if (!sign) {
    console.log(`Repacked AppImage, left unsigned: ${appImage}`);
    return {
      appImage,
      removedLibraries: hostLibraries,
      updatedGtkHook,
      signed: false,
    };
  }

  await run("pnpm", [
    "-F",
    "@anlg/desktop",
    "tauri",
    "signer",
    "sign",
    appImage,
  ]);

  const signatureStat = await stat(signature);
  if (signatureStat.size === 0) {
    throw new Error(`Regenerated AppImage signature is empty: ${signature}`);
  }

  console.log(`Repacked and re-signed AppImage: ${appImage}`);
  return {
    appImage,
    removedLibraries: hostLibraries,
    updatedGtkHook,
    signed: true,
  };
}

async function main() {
  const bundleDirectory = process.argv[2];
  if (!bundleDirectory) {
    throw new Error(
      "Usage: node scripts/repack-linux-appimage.mjs <appimage-bundle-directory>",
    );
  }

  await repackLinuxAppImage({
    bundleDirectory: path.resolve(bundleDirectory),
  });
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
