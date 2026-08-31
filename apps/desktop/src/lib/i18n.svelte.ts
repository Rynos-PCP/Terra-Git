// Lightweight i18n layer (no dependency): reactive language state + a t()
// lookup with {param} interpolation. A language change takes effect immediately
// (runes reactivity), no restart needed.
//
// Conventions:
// - NEVER call t() at module top level (the result would be frozen) — always in
//   the template, in $derived or inside functions.
// - Keys: "<area>.<slug>", parameters as {name} in the text.
import { de, type MessageKey } from "./messages/de";
import { en } from "./messages/en";

export type Lang = "de" | "en";
const STORAGE_KEY = "terra-git-lang";

/** English is the default; German systems start in German. */
function systemDefault(): Lang {
  if (typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("de")) {
    return "de";
  }
  return "en";
}

function storedLang(): Lang | null {
  try {
    const v = typeof localStorage !== "undefined" ? localStorage.getItem(STORAGE_KEY) : null;
    return v === "de" || v === "en" ? v : null;
  } catch {
    return null;
  }
}

export const i18n = $state({ lang: storedLang() ?? systemDefault() });

/**
 * Keeps `<html lang>` in step with the UI language. Without it the document
 * keeps whatever index.html hard-codes, so screen readers, hyphenation and
 * spellcheck follow a stale value instead of the language on screen.
 */
function syncDocumentLang(lang: Lang) {
  if (typeof document !== "undefined") document.documentElement.lang = lang;
}

// The start language is resolved above rather than picked, so apply it as well.
syncDocumentLang(i18n.lang);

export function setLang(lang: Lang) {
  i18n.lang = lang;
  syncDocumentLang(lang);
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    // Persistence is a convenience — the switch itself works regardless.
  }
}

const tables: Record<Lang, Partial<Record<MessageKey, string>>> = { de, en };

/**
 * Translates known backend error codes (`err.<code>` catalog entries).
 * Codes whose messages carry details (paths, branch names) deliberately have NO
 * entry — the backend's original message stays there.
 */
export function resolveErrorMessage(code: string | undefined, fallback: string): string {
  const key = `err.${code}`;
  if (code && key in de) {
    // Some codes carry a detail (e.g. the list of missing tools). The catalog
    // text can embed it through {detail} instead of losing it.
    return t(key as MessageKey, { detail: fallback });
  }
  return fallback;
}

/** Translates a key into the active language; {param} placeholders are replaced. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  let msg = tables[i18n.lang][key] ?? de[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      msg = msg.split(`{${k}}`).join(String(v));
    }
  }
  return msg;
}

/** Bases for which the catalog has BOTH plural forms (`<base>One`/`<base>Many`). */
type PluralBase = {
  [K in MessageKey]: K extends `${infer B}One`
    ? `${B}Many` extends MessageKey
      ? B
      : never
    : never;
}[MessageKey];

/**
 * Plural lookup: n === 1 -> `<base>One`, otherwise `<base>Many`. {n} and further
 * parameters are interpolated as in t(). Always create plural texts as a
 * One/Many pair — the type of `base` enforces the pair.
 */
export function tn(base: PluralBase, n: number, params?: Record<string, string | number>): string {
  return t(`${base}${n === 1 ? "One" : "Many"}` as MessageKey, { n, ...params });
}

export type { MessageKey };
