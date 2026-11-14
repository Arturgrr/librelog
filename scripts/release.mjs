#!/usr/bin/env node
// Release helpers for .github/workflows/release.yaml. Values arrive through
// environment variables and arguments, never through workflow expressions.

import {
  createHash,
  createPublicKey,
  verify as verifySignature,
} from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const packageJsonPath = join(root, "apps/desktop/package.json");
const changelogDir = join(root, "packages/changelog/content/desktop");

// Stable `X.Y.Z` or beta `X.Y.Z-beta.N`. Semver orders 0.1.0-beta.2 after
// 0.1.0-beta.1 and 0.1.0 after every 0.1.0-beta.N, which is what the updater
// compares.
const VERSION_PATTERN =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-beta\.(0|[1-9]\d*))?$/;

export const TAG_PREFIX = "desktop_v";

// A push to main releases whatever version apps/desktop/package.json holds;
// a manual run must name that same version.
export function resolveReleaseVersion({
  eventName,
  refName,
  inputVersion,
  packageVersion,
}) {
  if (eventName === "push" && refName !== "main") {
    throw new Error(`Releases run from pushes to main, not ${refName}.`);
  }
  const version = eventName === "push" ? packageVersion : inputVersion;
  if (!VERSION_PATTERN.test(version ?? "")) {
    throw new Error(
      `Expected a version such as 0.1.0-beta.1 or 0.1.0, got ${JSON.stringify(version ?? "")}.`,
    );
  }
  if (version !== packageVersion) {
    throw new Error(
      `apps/desktop/package.json is at ${packageVersion}. Bump it to ${version} on the release branch first.`,
    );
  }
  return version;
}

export function isBeta(version) {
  return version.includes("-beta.");
}

// RPM and dpkg would order 0.1.0-beta.1 after 0.1.0, so a package manager
// would treat the final release as a downgrade. A tilde sorts before
// everything in both, which keeps betas ahead of their final release.
export function linuxPackageVersion(version) {
  return version.replace("-beta.", "~beta.");
}

// Turns an in-app changelog entry into GitHub markdown: drops the frontmatter
// and renders <banner> tags, which GitHub would strip, as alerts.
export function changelogToGithubMarkdown(markdown) {
  const match = markdown.trim().match(/^---\s*\n[\s\S]*?\n---\s*\n([\s\S]*)$/);
  const body = (match ? match[1] : markdown).trim();
  return body.replace(
    /<banner(?:\s+title="([^"]*)")?(?:\s+variant="([^"]*)")?\s*>([\s\S]*?)<\/banner>/g,
    (_, title, variant, content) =>
      [
        variant === "warning" ? "> [!WARNING]" : "> [!NOTE]",
        ...(title ? [`> **${title}**`, ">"] : []),
        ...content
          .trim()
          .split("\n")
          .map((line) => (line ? `> ${line}` : ">")),
      ].join("\n"),
  );
}

export function releaseNotes({ version, repository, changelog }) {
  const dmg = (arch) => `Librelog_${version}_${arch}.dmg`;
  const rpm = `Librelog-${version}-1.x86_64.rpm`;
  const deb = `Librelog_${version}_amd64.deb`;
  const appImage = `Librelog_${version}_amd64.AppImage`;
  const beta = isBeta(version)
    ? [
        "> [!WARNING]",
        "> This is a beta. Expect rough edges and report anything that breaks. Your notes and recordings stay on your computer.",
        "",
      ]
    : [];

  return [
    ...beta,
    "## Download",
    "",
    "### macOS",
    "",
    `Download \`${dmg("aarch64")}\` below.`,
    "",
    "Requires a Mac with Apple silicon (M1 or later) and macOS 15 Sequoia or later. Intel Macs are not supported.",
    "",
    "### Linux (x86_64)",
    "",
    `- Fedora: download \`${rpm}\` and run \`sudo dnf install ./${rpm}\`.`,
    `- Ubuntu and Debian: download \`${deb}\` and run \`sudo apt install ./${deb}\`.`,
    `- Other distributions: download \`${appImage}\`, make it executable with \`chmod +x\`, and run it.`,
    "",
    "The AppImage updates itself from inside the app. RPM and deb installs do not: the app tells you when a new version is out and opens its release page, and you install the new package the same way.",
    "",
    `Read the [Linux install guide](https://github.com/${repository}/blob/main/docs/install-linux.mdx) for requirements and what is not available on Linux yet.`,
    "",
    "## First launch on macOS",
    "",
    "Librelog is not notarized by Apple yet, so macOS blocks the first launch:",
    "",
    "1. Open the DMG and drag **Librelog** to **Applications**.",
    "2. Open Librelog. When macOS says it could not verify Librelog, click **Done**.",
    "3. Open **System Settings → Privacy & Security**, scroll to **Security**, and click **Open Anyway** next to the Librelog message. Confirm with your password or Touch ID, then click **Open Anyway** again.",
    "",
    "If macOS says Librelog is damaged, or you prefer Terminal, remove the download quarantine instead:",
    "",
    "```sh",
    "xattr -dr com.apple.quarantine /Applications/Librelog.app",
    "```",
    "",
    "You only do this once. Later versions install from inside the app.",
    "",
    "## Verify the download",
    "",
    "In the folder that holds the downloaded files, run:",
    "",
    "```sh",
    "shasum -a 256 -c SHA256SUMS.txt --ignore-missing # macOS",
    "sha256sum -c SHA256SUMS.txt --ignore-missing # Linux",
    "```",
    "",
    "## Feedback",
    "",
    `Choose **Help → Report Bug** in Librelog, or [open an issue](https://github.com/${repository}/issues/new/choose). The app fills in your version, macOS version, and chip. Report security problems [privately](https://github.com/${repository}/security/advisories/new).`,
    "",
    "## What's new",
    "",
    changelogToGithubMarkdown(changelog),
    "",
  ].join("\n");
}

function signedAsset(assets, matches, description) {
  const file = assets.find((asset) => matches(asset.name));
  if (!file) {
    throw new Error(`Missing the ${description}.`);
  }
  const signature = assets.find(
    (asset) => asset.name === `${file.name}.sig`,
  )?.content;
  if (!signature) {
    throw new Error(`Missing the signature for ${file.name}.`);
  }
  return { name: file.name, signature: signature.trim() };
}

// Builds latest.json from the updater artifacts uploaded by the build jobs.
// URLs use the public download path so update checks do not count against the
// unauthenticated REST API limit.
//
// Linux lists only the AppImage, which the updater replaces in place. RPM and
// deb installs look up linux-x86_64-rpm or -deb first, fall back to
// linux-x86_64 and so learn that a version is out, but the app sends them to
// the release page instead of installing (plugins/updater2).
export function updaterManifest({
  version,
  notes,
  pubDate,
  repository,
  tag,
  assets,
}) {
  const entry = ({ name, signature }) => ({
    signature,
    url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(name)}`,
  });
  const mac = entry(
    signedAsset(
      assets,
      (name) => name.endsWith("_aarch64.app.tar.gz"),
      "aarch64 updater archive",
    ),
  );
  const appImage = entry(
    signedAsset(
      assets,
      (name) => name.endsWith("_amd64.AppImage"),
      "x86_64 AppImage",
    ),
  );
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: {
      "darwin-aarch64": mac,
      "darwin-aarch64-app": mac,
      "linux-x86_64": appImage,
      "linux-x86_64-appimage": appImage,
    },
  };
}

function minisignLines(base64, kind) {
  const lines = Buffer.from(base64.trim(), "base64")
    .toString("utf8")
    .split("\n")
    .map((line) => line.trim());
  if (!lines[0]?.startsWith("untrusted comment:")) {
    throw new Error(`The ${kind} is not a minisign ${kind}.`);
  }
  return lines;
}

// Checks a Tauri updater signature (a base64-encoded minisign signature)
// against the base64-encoded public key in tauri.conf.json, as the updater
// does before it installs.
export function verifyUpdaterSignature({ data, signature, publicKey }) {
  const keyBytes = Buffer.from(minisignLines(publicKey, "key")[1], "base64");
  const [, signatureLine, trustedLine, globalLine] = minisignLines(
    signature,
    "signature",
  );
  const signatureBytes = Buffer.from(signatureLine, "base64");
  if (keyBytes.length !== 42 || signatureBytes.length !== 74) {
    throw new Error("Malformed minisign key or signature.");
  }
  if (!keyBytes.subarray(2, 10).equals(signatureBytes.subarray(2, 10))) {
    throw new Error("The signature was made with a different key.");
  }
  const key = createPublicKey({
    key: {
      kty: "OKP",
      crv: "Ed25519",
      x: keyBytes.subarray(10).toString("base64url"),
    },
    format: "jwk",
  });
  const algorithm = signatureBytes.subarray(0, 2).toString("latin1");
  const message =
    algorithm === "ED" ? createHash("blake2b512").update(data).digest() : data;
  const fileSignature = signatureBytes.subarray(10);
  if (!verifySignature(null, message, key, fileSignature)) {
    throw new Error("The signature does not match the file.");
  }
  const trustedComment = trustedLine?.replace(/^trusted comment: /, "") ?? "";
  if (
    !verifySignature(
      null,
      Buffer.concat([fileSignature, Buffer.from(trustedComment)]),
      key,
      Buffer.from(globalLine ?? "", "base64"),
    )
  ) {
    throw new Error("The signature's trusted comment was altered.");
  }
}

function readPackageVersion() {
  return JSON.parse(readFileSync(packageJsonPath, "utf8")).version;
}

function changelogPath(version) {
  return join(changelogDir, `${version}.md`);
}

function main([command, ...args]) {
  const env = process.env;
  switch (command) {
    case "version": {
      const version = resolveReleaseVersion({
        eventName: env.EVENT_NAME,
        refName: env.REF_NAME,
        inputVersion: env.INPUT_VERSION,
        packageVersion: readPackageVersion(),
      });
      if (!existsSync(changelogPath(version))) {
        throw new Error(
          `Add release notes at packages/changelog/content/desktop/${version}.md first.`,
        );
      }
      process.stdout.write(`${version}\n`);
      return;
    }
    case "notes": {
      const [version] = args;
      process.stdout.write(
        releaseNotes({
          version,
          repository: env.GITHUB_REPOSITORY,
          changelog: readFileSync(changelogPath(version), "utf8"),
        }),
      );
      return;
    }
    case "updater-manifest": {
      const [version, directory] = args;
      const assets = readdirSync(directory).map((name) => ({
        name,
        content: name.endsWith(".sig")
          ? readFileSync(join(directory, name), "utf8")
          : undefined,
      }));
      const manifest = updaterManifest({
        version,
        notes: `See https://github.com/${env.GITHUB_REPOSITORY}/releases/tag/${TAG_PREFIX}${version}`,
        pubDate: new Date().toISOString(),
        repository: env.GITHUB_REPOSITORY,
        tag: `${TAG_PREFIX}${version}`,
        assets,
      });
      process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
      return;
    }
    case "linux-package-version": {
      const [version] = args;
      process.stdout.write(`${linuxPackageVersion(version)}\n`);
      return;
    }
    case "verify-updater-signature": {
      const [file, signatureFile, configFile] = args;
      verifyUpdaterSignature({
        data: readFileSync(file),
        signature: readFileSync(signatureFile, "utf8"),
        publicKey: JSON.parse(readFileSync(configFile, "utf8")).plugins.updater
          .pubkey,
      });
      process.stdout.write(`${file}: updater signature verified\n`);
      return;
    }
    default:
      throw new Error(
        "Usage: release.mjs version | notes <version> | updater-manifest <version> <dir> | linux-package-version <version> | verify-updater-signature <file> <sig> <tauri.conf.json>",
      );
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}
