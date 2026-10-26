// The error registry: ONE place that says, per error code,
// which ways out the error toast offers. Title and text come from the catalogs
// (`errt.<code>` / `err.<code>`); the actions here are the static ones — the
// situational ones (open the conflict workshop when a running operation has
// conflicts, "stash and switch" with its target) stay `ui.errorAction`,
// because they depend on state the code alone does not know.
import type { ErrorCode } from "./errorCodes";
import type { MessageKey } from "./i18n.svelte";

/** A way out the toast can offer for an error code. */
export type ErrorActionId =
  | "manageBackups"
  | "raiseLimit"
  | "openAccounts"
  | "openSshKeys"
  | "openRemotes"
  | "installLfs"
  | "retry"
  | "openIdentity"
  | "branchHere"
  | "removeLock"
  | "trustFolder"
  | "enableLongPaths"
  | "publishAgain"
  | "unsetUpstream"
  | "unshallow"
  | "mergeUnrelated"
  | "commitUnsigned"
  | "openSigning"
  | "deleteGoneBranch";

/** Button label per action. */
export const ERROR_ACTION_LABELS: Record<ErrorActionId, MessageKey> = {
  manageBackups: "state.manageBackups",
  raiseLimit: "state.raiseLimit",
  openAccounts: "errorAction.openAccounts",
  openSshKeys: "errorAction.openSshKeys",
  openRemotes: "errorAction.openRemotes",
  installLfs: "errorAction.installLfs",
  retry: "errorAction.retry",
  openIdentity: "errorAction.openIdentity",
  branchHere: "errorAction.branchHere",
  removeLock: "errorAction.removeLock",
  trustFolder: "errorAction.trustFolder",
  enableLongPaths: "errorAction.enableLongPaths",
  publishAgain: "errorAction.publishAgain",
  unsetUpstream: "errorAction.unsetUpstream",
  unshallow: "errorAction.unshallow",
  mergeUnrelated: "errorAction.mergeUnrelated",
  commitUnsigned: "errorAction.commitUnsigned",
  openSigning: "errorAction.openSigning",
  deleteGoneBranch: "errorAction.deleteGoneBranch",
};

/** Static actions per code. Codes without an entry offer none (yet). */
export const ERROR_ACTIONS: Partial<Record<ErrorCode, ErrorActionId[]>> = {
  discard_store_full: ["manageBackups", "raiseLimit"],
  auth_failed: ["openAccounts"],
  forbidden: ["openAccounts"],
  invalid_token: ["openAccounts"],
  no_account: ["openAccounts"],
  ssh_auth: ["openSshKeys"],
  repo_not_found: ["openRemotes"],
  no_remote: ["openRemotes"],
  lfs_missing: ["installLfs", "retry"],
  network: ["retry"],
  identity_missing: ["openIdentity"],
  tls_cert: ["openAccounts"],
  detached_push: ["branchHere"],
  index_locked: ["removeLock"],
  unsafe_repository: ["trustFolder"],
  path_too_long: ["enableLongPaths"],
  upstream_gone: ["publishAgain", "unsetUpstream", "deleteGoneBranch"],
  shallow_rejected: ["unshallow"],
  unrelated_histories: ["mergeUnrelated"],
  hook_failed: ["retry"],
  signing_failed: ["commitUnsigned", "openSigning"],
};

/** The actions the toast offers for `code`. */
export function errorActions(code: string | undefined): ErrorActionId[] {
  return (code && ERROR_ACTIONS[code as ErrorCode]) || [];
}

/** The catalog key of the short title for `code` (unknown codes fall back). */
export function errorTitleKey(code: string | undefined, has: (key: string) => boolean): MessageKey {
  const key = `errt.${code}`;
  return (code && has(key) ? key : "errt.unknown") as MessageKey;
}
