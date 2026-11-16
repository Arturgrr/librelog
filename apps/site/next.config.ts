import type { NextConfig } from "next";
import { PHASE_DEVELOPMENT_SERVER } from "next/constants";

const PRODUCTION_URL = "https://arturgrr.github.io/librelog/";

export default function config(phase: string): NextConfig {
  const configured = process.env.SITE_URL || PRODUCTION_URL;
  const siteUrl =
    phase === PHASE_DEVELOPMENT_SERVER
      ? "http://127.0.0.1:4321/"
      : configured.endsWith("/")
        ? configured
        : `${configured}/`;
  const basePath = new URL(siteUrl).pathname.replace(/\/$/, "");

  return {
    output: "export",
    basePath,
    trailingSlash: true,
    images: { unoptimized: true },
    transpilePackages: ["@anlg/utils"],
    env: { SITE_URL: siteUrl, BASE_PATH: basePath },
    experimental: { globalNotFound: true },
  };
}
