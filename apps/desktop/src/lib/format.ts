import { i18n, t } from "./i18n.svelte";

/** Relative time in the active language ("3 h ago"). */
export function timeAgo(unixSeconds: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (diff < 60) return t("time.justNow");
  const min = Math.floor(diff / 60);
  if (min < 60) return t("time.minutes", { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t("time.hours", { n: h });
  const d = Math.floor(h / 24);
  if (d < 30) return d === 1 ? t("time.yesterday") : t("time.days", { n: d });
  const date = new Date(unixSeconds * 1000);
  return date.toLocaleDateString(i18n.lang === "de" ? "de-DE" : "en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/** Deterministic avatar colour from the author name.
 *  Curated hues along the graph palette instead of arbitrary values, so avatars
 *  match the theme and harmonize with each other. */
const AVATAR_HUES = [165, 208, 265, 38, 335, 100, 185, 18];

export function avatarColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash * 31 + name.charCodeAt(i)) | 0;
  }
  const hue = AVATAR_HUES[Math.abs(hash) % AVATAR_HUES.length];
  return `hsl(${hue}, 42%, 42%)`;
}

/** Initials for avatar circles ("Ada Lovelace" -> "AL"). */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/** Human-readable byte size (1 KB = 1024). */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  // Cascading rounding: 1048575 yields v = 1023.999 KB, which would round to
  // "1024". In that case move up a unit so "1024 KB" never appears instead of
  // "1.0 MB".
  let decimals = v < 10 ? 1 : 0;
  while (Number(v.toFixed(decimals)) >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
    decimals = v < 10 ? 1 : 0;
  }
  return `${v.toFixed(decimals)} ${units[i]}`;
}

/** Derives the default folder name from a clone URL: the last path segment
 *  without a trailing ".git". Covers the scp form (git@host:owner/repo.git) and
 *  http(s)/ssh URLs. An empty string when nothing sensible can be derived. */
export function deriveCloneName(url: string): string {
  return (
    url
      .trim()
      .replace(/\/+$/, "")
      .split("/")
      .pop()
      ?.replace(/\.git$/, "") ?? ""
  );
}

/** Shortens long paths in the middle ("C:/Users/…/foo/bar"). */
export function shortenPath(path: string, max = 60): string {
  if (path.length <= max) return path;
  const parts = path.split("/");
  if (parts.length <= 3) return path;

  const head = parts.slice(0, 2).join("/");
  const tail: string[] = [parts[parts.length - 1]];
  // Add segments from the back as long as they fit the budget.
  for (let i = parts.length - 2; i >= 2; i--) {
    const candidate = `${head}/…/${parts[i]}/${tail.join("/")}`;
    if (candidate.length > max) break;
    tail.unshift(parts[i]);
  }
  return `${head}/…/${tail.join("/")}`;
}
