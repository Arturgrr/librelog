import assert from "node:assert/strict";
import { constants } from "node:fs";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { repackLinuxAppImage } from "./repack-linux-appimage.mjs";

async function createFixture(t) {
  const directory = await mkdtemp(
    path.join(os.tmpdir(), "librelog-appimage-repack-"),
  );
  t.after(() => rm(directory, { force: true, recursive: true }));

  const appDirectory = path.join(directory, "Librelog.AppDir");
  const libraries = path.join(appDirectory, "usr", "lib");
  const appImage = path.join(directory, "Librelog_0.8.1-beta.1_amd64.AppImage");
  const gtkHook = path.join(
    appDirectory,
    "apprun-hooks",
    "linuxdeploy-plugin-gtk.sh",
  );
  const plugin = path.join(
    directory,
    "tools",
    "linuxdeploy-plugin-appimage.AppImage",
  );

  await mkdir(path.join(libraries, "gtk-3.0"), { recursive: true });
  await mkdir(path.dirname(gtkHook), { recursive: true });
  await mkdir(path.dirname(plugin), { recursive: true });
  await writeFile(
    gtkHook,
    "#!/usr/bin/env bash\nexport GDK_BACKEND=x11 # Crash with Wayland backend on Wayland\n",
  );
  for (const name of [
    "libgtk-3.so.0",
    "libpulse.so.0",
    "libpipewire-0.3.so.0",
    "libspa-support.so",
    "gtk-3.0/libwayland-client.so.0",
  ]) {
    await writeFile(path.join(libraries, name), name);
  }
  await writeFile(appImage, "original");
  await writeFile(`${appImage}.sig`, "stale-signature");
  await writeFile(plugin, "plugin");
  await chmod(plugin, 0o755);

  const exists = (file) =>
    access(path.join(libraries, file), constants.F_OK).then(
      () => true,
      () => false,
    );
  return { directory, appImage, gtkHook, plugin, exists };
}

function fakeRun(fixture, commands) {
  return async (command, args) => {
    commands.push([command, ...args]);
    if (command === fixture.plugin) {
      await writeFile(fixture.appImage, "repacked");
    } else {
      await writeFile(`${fixture.appImage}.sig`, "fresh-signature");
    }
  };
}

test("the repacked AppImage uses Fedora's PipeWire and Wayland and leaves the GDK backend to the app", async (t) => {
  const fixture = await createFixture(t);
  const commands = [];

  await repackLinuxAppImage({
    bundleDirectory: fixture.directory,
    arch: "x86_64",
    pluginPath: fixture.plugin,
    sign: true,
    run: fakeRun(fixture, commands),
  });

  assert.equal(await fixture.exists("libpipewire-0.3.so.0"), false);
  assert.equal(await fixture.exists("libspa-support.so"), false);
  assert.equal(await fixture.exists("gtk-3.0/libwayland-client.so.0"), false);
  assert.equal(await fixture.exists("libpulse.so.0"), true);
  assert.equal(await fixture.exists("libgtk-3.so.0"), true);
  assert.doesNotMatch(await readFile(fixture.gtkHook, "utf8"), /GDK_BACKEND/);
  assert.equal(await readFile(fixture.appImage, "utf8"), "repacked");
  assert.equal(
    await readFile(`${fixture.appImage}.sig`, "utf8"),
    "fresh-signature",
  );
  assert.deepEqual(commands.at(-1), [
    "pnpm",
    "-F",
    "@anlg/desktop",
    "tauri",
    "signer",
    "sign",
    fixture.appImage,
  ]);
});

test("an AppImage repacked without the signing key ships no stale signature", async (t) => {
  const fixture = await createFixture(t);

  await repackLinuxAppImage({
    bundleDirectory: fixture.directory,
    arch: "x86_64",
    pluginPath: fixture.plugin,
    sign: false,
    run: fakeRun(fixture, []),
  });

  await assert.rejects(access(`${fixture.appImage}.sig`, constants.F_OK));
});
