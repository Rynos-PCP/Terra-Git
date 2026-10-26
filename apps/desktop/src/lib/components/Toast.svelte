<script lang="ts">
  import { api } from "../api";
  import { offerConflictWorkshop } from "../conflictOffer";
  import { ERROR_ACTION_LABELS, errorActions, type ErrorActionId } from "../errorRegistry";
  import { t } from "../i18n.svelte";
  import {
    clearToast,
    openConflicts,
    openStashes,
    runErrorAction,
    showError,
    stashAndSwitch,
    ui,
  } from "../state.svelte";
  import { tooltip } from "../tooltip";
  import Icon from "./Icon.svelte";

  // Info toasts disappear after the configured duration (0 = never); errors stay until clicked.
  $effect(() => {
    if (ui.info && ui.toastDuration > 0) {
      const timer = setTimeout(() => (ui.info = null), ui.toastDuration * 1000);
      return () => clearTimeout(timer);
    }
  });

  // Conflict errors offer the jump into the workshop — reactively, because the
  // status (opState, conflicted files) often arrives AFTER the error.
  const offerWorkshop = $derived(
    !!ui.error &&
      offerConflictWorkshop(
        ui.errorAction?.kind ?? null,
        ui.status?.opState ?? "clean",
        (ui.status?.unstaged ?? []).filter((e) => e.kind === "conflicted").length,
        ui.view,
      ),
  );

  // Blocked branch switch: the toast carries the way out (stash + switch)
  // including the target. No gate of its own needed — the action applies exactly
  // to the error that set it, and is cleared away with it.
  const stashSwitch = $derived(
    ui.error && ui.errorAction?.kind === "stashSwitch" ? ui.errorAction.target : null,
  );

  // The static ways out of the error registry.
  const registryActions = $derived(
    ui.error
      ? errorActions(ui.errorMeta?.code).filter(
          (id) =>
            (id !== "retry" || ui.canRetry) &&
            (id !== "mergeUnrelated" || ui.canMergeUnrelated) &&
            (id !== "commitUnsigned" || ui.canCommitUnsigned),
        )
      : [],
  );

  // Heading "REPO : code — title" (user wish 2026-10-12).
  const heading = $derived(
    ui.errorMeta ? `${ui.errorMeta.repo} : ${ui.errorMeta.code} — ${ui.errorMeta.title}` : "",
  );

  /** Every registry action closes the toast — it has done its job. */
  function runAction(id: ErrorActionId) {
    void runErrorAction(id);
  }

  let copied = $state(false);

  /** Heading, text and git output in one piece — for a bug report or a search. */
  async function copyDetails() {
    const parts = [heading, ui.error ?? "", ui.errorMeta?.detail ?? ""].filter(Boolean);
    try {
      await navigator.clipboard.writeText(parts.join("\n\n"));
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // Clipboard denied: nothing to report beyond the toast already shown.
    }
  }

  async function openLog() {
    try {
      await api.openLogs();
    } catch (e) {
      showError(e);
    }
  }

  // Action buttons only while NO focus trap is open — a modal or the command
  // palette. The toast lies visibly above them, but both trap Tab cyclically and
  // thereby lock it away from the keyboard and screen readers.
  // A button only the mouse can reach is not an action but a trap.
  // The message itself stays, and the buttons come back as soon as the trap is
  // closed (error toasts stay until they are dismissed).
  const actionable = $derived(!ui.modal && !ui.paletteOpen);
  const hasActions = $derived(
    actionable &&
      (offerWorkshop ||
        !!stashSwitch ||
        ui.errorAction?.kind === "stashes" ||
        registryActions.length > 0),
  );
</script>

{#if ui.error}
  <!-- Live region ONLY on the message: role=alert is implicitly aria-atomic,
       and the workshop button only appears once the status arrives after
       the error — if the region sat on the container, the insertion would
       announce the complete message a second time. -->
  <div class="toast error">
    <!-- The head never scrolls: title, copy, log and close stay reachable
         however long git's output is (user wish: errors bounded to the window). -->
    <div class="head">
      <span class="ico"><Icon name="alert" size={15} /></span>
      <span class="title">{heading}</span>
      <button
        class="ghost icon"
        onclick={copyDetails}
        aria-label={t("toast.copyDetails")}
        use:tooltip={copied ? t("toast.detailsCopied") : t("toast.copyDetails")}
      >
        <Icon name={copied ? "check" : "copy"} size={13} />
      </button>
      <button
        class="ghost icon"
        onclick={openLog}
        aria-label={t("toast.openLog")}
        use:tooltip={t("toast.openLog")}
      >
        <Icon name="file" size={13} />
      </button>
      <button class="ghost icon" onclick={clearToast} aria-label={t("common.close")}>
        <Icon name="x" size={13} />
      </button>
    </div>
    <div class="body">
      <p class="msg" role="alert" aria-live="assertive">{ui.error}</p>
      {#if ui.errorMeta?.detail}
        <!-- Focusable so the keyboard can scroll it. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre class="detail" tabindex="0" aria-label={t("toast.details")}>{ui.errorMeta.detail}</pre>
      {/if}
    </div>
    {#if hasActions}
      <div class="actions">
        {#if offerWorkshop}
          <button class="primary act" onclick={() => openConflicts()}
            >{t("conflictws.openLong")}</button
          >
        {/if}
        {#if stashSwitch}
          <button class="primary act" onclick={() => stashAndSwitch(stashSwitch)}>
            {t("state.stashAndSwitch")}
          </button>
        {/if}
        {#if ui.errorAction?.kind === "stashes"}
          <button class="primary act" onclick={openStashes}>{t("state.openStashes")}</button>
        {/if}
        {#each registryActions as id, i (id)}
          <button
            class="act"
            class:primary={i === 0}
            data-testid={`error-action-${id}`}
            onclick={() => runAction(id)}
          >
            {t(ERROR_ACTION_LABELS[id])}
          </button>
        {/each}
      </div>
    {/if}
  </div>
{:else if ui.info}
  <div class="toast">
    <div class="head">
      <span class="ico"><Icon name="check" size={15} /></span>
      <span class="msg info" role="status" aria-live="polite">{ui.info}</span>
      <button class="ghost icon" onclick={clearToast} aria-label={t("common.close")}>
        <Icon name="x" size={13} />
      </button>
    </div>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: max-content;
    max-width: min(720px, calc(100vw - 48px));
    /* Never past the window edge: the body scrolls instead (user wish
       2026-10-12 — long git output used to push the toast off the top). */
    max-height: calc(100vh - 32px);
    padding: 10px 10px 10px 14px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-menu);
    z-index: 100;
    animation: toast-in 0.2s cubic-bezier(0.2, 0.8, 0.3, 1);
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(8px);
    }
  }

  .toast.error {
    border-color: var(--border-strong);
    border-left-color: var(--deleted);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .ico {
    display: inline-flex;
    color: var(--accent);
    flex-shrink: 0;
  }

  .toast.error .ico {
    color: var(--deleted);
  }

  .title {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .icon {
    flex-shrink: 0;
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow-y: auto;
  }

  .msg {
    margin: 0;
    user-select: text;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .msg.info {
    flex: 1;
    white-space: normal;
  }

  .detail {
    margin: 0;
    max-height: min(40vh, 320px);
    overflow: auto;
    padding: 8px 10px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 4px);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    flex-shrink: 0;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    flex-shrink: 0;
  }

  /* Action buttons must not shrink in the flexing toast. */
  .act {
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>
