import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import test from "node:test";

import {
  linuxPackageVersion,
  releaseNotes,
  resolveReleaseVersion,
  updaterManifest,
  verifyUpdaterSignature,
} from "./release.mjs";

test("release versions come from the app on main or from the input", () => {
  assert.equal(
    resolveReleaseVersion({
      eventName: "push",
      refName: "main",
      packageVersion: "0.1.0-beta.2",
    }),
    "0.1.0-beta.2",
  );
  assert.throws(() =>
    resolveReleaseVersion({
      eventName: "push",
      refName: "main",
      packageVersion: "0.1.0-rc.1",
    }),
  );
  assert.equal(
    resolveReleaseVersion({
      eventName: "workflow_dispatch",
      inputVersion: "1.2.3",
      packageVersion: "1.2.3",
    }),
    "1.2.3",
  );

  for (const inputVersion of [
    "v0.1.0",
    "0.1",
    "0.1.0-beta",
    "0.1.0-beta.01",
    "0.1.0-rc.1",
    "0.1.0-beta.1\necho",
    "$(id)",
  ]) {
    assert.throws(() =>
      resolveReleaseVersion({
        eventName: "workflow_dispatch",
        inputVersion,
        packageVersion: inputVersion,
      }),
    );
  }

  assert.throws(
    () =>
      resolveReleaseVersion({
        eventName: "push",
        refName: "integration/local-first",
        packageVersion: "0.1.0-beta.1",
      }),
    /pushes to main/,
  );
  assert.throws(
    () =>
      resolveReleaseVersion({
        eventName: "workflow_dispatch",
        inputVersion: "0.1.0-beta.2",
        packageVersion: "0.1.0-beta.1",
      }),
    /Bump it to 0\.1\.0-beta\.2/,
  );
});

test("release notes explain the macOS 15 first launch, list the Linux packages and include the changelog", () => {
  const notes = releaseNotes({
    version: "0.1.0-beta.1",
    repository: "Arturgrr/librelog",
    changelog:
      '---\ndate: "2026-11-06"\nsummary: "First beta."\n---\n\n<banner title="Welcome">\nLine one.\n\nLine two.\n</banner>\n\n## Hello\n',
  });

  assert.match(notes, /This is a beta/);
  assert.match(notes, /Librelog_0\.1\.0-beta\.1_aarch64\.dmg/);
  assert.match(notes, /Apple silicon/);
  assert.doesNotMatch(notes, /_x64\.dmg/);
  assert.match(notes, /Privacy & Security.*Open Anyway/);
  assert.doesNotMatch(notes, /right-click/i);
  assert.match(
    notes,
    /sudo dnf install \.\/Librelog-0\.1\.0-beta\.1-1\.x86_64\.rpm/,
  );
  assert.match(
    notes,
    /sudo apt install \.\/Librelog_0\.1\.0-beta\.1_amd64\.deb/,
  );
  assert.match(notes, /Librelog_0\.1\.0-beta\.1_amd64\.AppImage/);
  assert.match(
    notes,
    /## What's new\n\n> \[!NOTE\]\n> \*\*Welcome\*\*\n>\n> Line one\.\n>\n> Line two\.\n\n## Hello\n$/,
  );
  assert.doesNotMatch(notes, /summary:|<banner/);
  assert.doesNotMatch(
    releaseNotes({ version: "0.1.0", repository: "a/b", changelog: "" }),
    /This is a beta/,
  );
});

const manifestAssets = [
  { name: "Librelog_0.1.0-beta.1_aarch64.dmg" },
  { name: "Librelog_0.1.0-beta.1_aarch64.app.tar.gz" },
  {
    name: "Librelog_0.1.0-beta.1_aarch64.app.tar.gz.sig",
    content: "arm-signature\n",
  },
  { name: "Librelog_0.1.0-beta.1_amd64.AppImage" },
  {
    name: "Librelog_0.1.0-beta.1_amd64.AppImage.sig",
    content: "appimage-signature\n",
  },
  { name: "Librelog_0.1.0-beta.1_amd64.deb" },
  { name: "Librelog-0.1.0-beta.1-1.x86_64.rpm" },
];

test("the updater manifest updates Macs and AppImages but leaves RPM and deb installs to the release page", () => {
  const manifest = updaterManifest({
    version: "0.1.0-beta.1",
    notes: "notes",
    pubDate: "2026-11-06T00:00:00.000Z",
    repository: "Arturgrr/librelog",
    tag: "desktop_v0.1.0-beta.1",
    assets: manifestAssets,
  });

  const download =
    "https://github.com/Arturgrr/librelog/releases/download/desktop_v0.1.0-beta.1";
  const arm = {
    signature: "arm-signature",
    url: `${download}/Librelog_0.1.0-beta.1_aarch64.app.tar.gz`,
  };
  const appImage = {
    signature: "appimage-signature",
    url: `${download}/Librelog_0.1.0-beta.1_amd64.AppImage`,
  };
  assert.deepEqual(manifest, {
    version: "0.1.0-beta.1",
    notes: "notes",
    pub_date: "2026-11-06T00:00:00.000Z",
    platforms: {
      "darwin-aarch64": arm,
      "darwin-aarch64-app": arm,
      "linux-x86_64": appImage,
      "linux-x86_64-appimage": appImage,
    },
  });
});

test("the updater manifest refuses to publish without every signed artifact", () => {
  const base = {
    version: "0.1.0-beta.1",
    notes: "",
    pubDate: "",
    repository: "a/b",
    tag: "t",
  };
  const without = (pattern) =>
    manifestAssets.filter((asset) => !pattern.test(asset.name));
  assert.throws(
    () => updaterManifest({ ...base, assets: without(/app\.tar\.gz/) }),
    /aarch64 updater archive/,
  );
  assert.throws(
    () => updaterManifest({ ...base, assets: without(/app\.tar\.gz\.sig$/) }),
    /signature for Librelog_0\.1\.0-beta\.1_aarch64/,
  );
  assert.throws(
    () => updaterManifest({ ...base, assets: without(/AppImage/) }),
    /x86_64 AppImage/,
  );
  assert.throws(
    () => updaterManifest({ ...base, assets: without(/AppImage\.sig$/) }),
    /signature for Librelog_0\.1\.0-beta\.1_amd64\.AppImage/,
  );
});

test("Linux packages order a beta before its final release", () => {
  assert.equal(linuxPackageVersion("0.8.1-beta.1"), "0.8.1~beta.1");
  assert.equal(linuxPackageVersion("0.8.1"), "0.8.1");
});

test("a repacked AppImage must carry a signature the updater accepts", () => {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const keyId = Buffer.from("0123456789abcdef", "hex");
  const rawPublicKey = Buffer.from(
    publicKey.export({ format: "jwk" }).x,
    "base64url",
  );
  const minisign = (lines) =>
    Buffer.from(`${lines.join("\n")}\n`).toString("base64");
  const data = Buffer.from("AppImage bytes");
  const fileSignature = sign(
    null,
    createHash("blake2b512").update(data).digest(),
    privateKey,
  );
  const trustedComment = "timestamp:1\tfile:Librelog.AppImage";
  const signature = minisign([
    "untrusted comment: signature from tauri secret key",
    Buffer.concat([Buffer.from("ED"), keyId, fileSignature]).toString("base64"),
    `trusted comment: ${trustedComment}`,
    sign(
      null,
      Buffer.concat([fileSignature, Buffer.from(trustedComment)]),
      privateKey,
    ).toString("base64"),
  ]);
  const key = minisign([
    "untrusted comment: minisign public key",
    Buffer.concat([Buffer.from("Ed"), keyId, rawPublicKey]).toString("base64"),
  ]);

  verifyUpdaterSignature({ data, signature, publicKey: key });
  assert.throws(
    () =>
      verifyUpdaterSignature({
        data: Buffer.from("repacked without signing"),
        signature,
        publicKey: key,
      }),
    /does not match/,
  );
});
