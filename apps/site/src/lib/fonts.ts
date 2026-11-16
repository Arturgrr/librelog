import localFont from "next/font/local";

// Self-hosted, so the page asks no other server for fonts. Licenses sit next to the files.
const bricolage = localFont({
  src: "../fonts/bricolage-grotesque-latin-wght-normal.woff2",
  weight: "200 800",
  variable: "--font-bricolage",
  display: "swap",
});

const instrument = localFont({
  src: "../fonts/instrument-sans-latin-wght-normal.woff2",
  weight: "400 700",
  variable: "--font-instrument",
  display: "swap",
});

const jetbrains = localFont({
  src: "../fonts/jetbrains-mono-latin-wght-normal.woff2",
  weight: "100 800",
  variable: "--font-jetbrains",
  display: "swap",
});

const caveat = localFont({
  src: "../fonts/caveat-latin-wght-normal.woff2",
  weight: "400 700",
  variable: "--font-caveat",
  display: "swap",
});

export const fontVariables = [
  bricolage.variable,
  instrument.variable,
  jetbrains.variable,
  caveat.variable,
].join(" ");
