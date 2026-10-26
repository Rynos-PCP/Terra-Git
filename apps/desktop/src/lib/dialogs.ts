// The app's confirmation questions, in one place: the native dialog of the
// dialog plugin. The E2E tests start the app with
// TERRA_GIT_E2E_ACCEPT_DIALOGS set (honoured by debug builds only) — WebDriver
// cannot reach a native dialog, and Tauri locks its IPC objects against
// patching — so then every question is answered with OK and recorded in
// `window.__e2eDialogs`.
import { invoke } from "@tauri-apps/api/core";
import { confirm as nativeConfirm } from "@tauri-apps/plugin-dialog";

let autoAccept: Promise<boolean> | null = null;

/** Asked once per window; anything but a clear yes means "ask the user". */
function acceptsByItself(): Promise<boolean> {
  autoAccept ??= invoke<boolean>("e2e_accept_dialogs")
    .then((yes) => yes === true)
    .catch(() => false);
  return autoAccept;
}

export async function confirm(
  message: string,
  options?: Parameters<typeof nativeConfirm>[1],
): Promise<boolean> {
  if (await acceptsByItself()) {
    const w = window as unknown as { __e2eDialogs?: string[] };
    (w.__e2eDialogs ??= []).push(message);
    return true;
  }
  return nativeConfirm(message, options);
}
