import desktop from "../../../desktop/package.json";

import { en } from "~/i18n/en";
import { ptBR } from "~/i18n/pt-BR";

export const SITE_URL =
  process.env.SITE_URL ?? "https://arturgrr.github.io/librelog/";

export const VERSION = desktop.version.replace(/^(\d+\.\d+).*$/, "$1");

export const LANGS = {
  en: { code: "en", path: "/", dict: en },
  pt: { code: "pt-BR", path: "/pt/", dict: ptBR },
} as const;

export type Lang = keyof typeof LANGS;

export const otherLang = (lang: Lang): Lang => (lang === "en" ? "pt" : "en");

export const withVersion = (text: string) =>
  text.replaceAll("{version}", VERSION);

// Sends a Portuguese-speaking first visit to /pt/, once; a choice made with the language link wins.
export const LANG_REDIRECT =
  'try{if(!localStorage.getItem("librelog-lang")&&/^pt\\b/i.test(navigator.language))location.replace("pt/")}catch(e){}';
