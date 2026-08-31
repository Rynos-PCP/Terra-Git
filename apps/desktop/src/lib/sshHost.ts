/** Extracts the host and (only for ssh://) the port from a remote URL.
 *  Supports the scp form `git@host:path`, `ssh://[user@]host[:port]/path`
 *  and `host:path`.
 *  - ssh:// form: IPv6 literals in square brackets are recognized and returned
 *    WITHOUT the brackets (the bare host); an optional `:port` (a number) is
 *    read out. The known_hosts form (`[host]:port`) is built by the backend.
 *  - scp form: the `:` separates the path, there is NO port here. IPv6 cannot
 *    be expressed in the scp form (the colon is ambiguous) and is therefore
 *    not supported.
 *  Returns null for HTTP(S) or unclear URLs. */
export function parseSshHost(url: string): { host: string; port: number | null } | null {
  const u = url.trim();
  if (/^https?:\/\//i.test(u)) return null;
  // ssh://[user@](host|[ipv6])[:port]/path — the port only here (a number).
  const ssh = u.match(/^ssh:\/\/(?:[^@/]+@)?(\[[0-9a-fA-F:]+\]|[^/:]+)(?::(\d+))?/i);
  if (ssh) {
    // IPv6 literal: remove the outer brackets (the bare host; the backend forms
    // `[host]:port` from it for known_hosts when needed).
    const raw = ssh[1];
    const host = raw.startsWith("[") && raw.endsWith("]") ? raw.slice(1, -1) : raw;
    return { host: host.toLowerCase(), port: ssh[2] ? Number(ssh[2]) : null };
  }
  // scp form: [user@]host:path (the ':' separates the path, no port; IPv6 n/a).
  const scp = u.match(/^(?:[^@/]+@)?([^/:]+):/);
  if (scp) return { host: scp[1].toLowerCase(), port: null };
  return null;
}
