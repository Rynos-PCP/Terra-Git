<script lang="ts">
  import { offerConflictWorkshop } from "../conflictOffer";
  import { t } from "../i18n.svelte";
  import { clearToast, openConflicts, openStashes, stashAndSwitch, ui } from "../state.svelte";
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

  // Action buttons only while NO focus trap is open — a modal or the command
  // palette. The toast lies visibly above them, but both trap Tab cyclically and
  // thereby lock it away from the keyboard and screen readers.
  // A button only the mouse can reach is not an action but a trap.
  // The message itself stays, and the buttons come back as soon as the trap is
  // closed (error toasts stay until they are dismissed).
  const actionable = $derived(!ui.modal && !ui.paletteOpen);
</script>

{#if ui.error || ui.info}
  <!-- Live region ONLY on the message: role=alert is implicitly aria-atomic,
       and the workshop button only appears once the status arrives after
       the error — if the region sat on the container, the insertion would
       announce the complete message a second time. -->
  <div class="toast" class:error={!!ui.error}>
    <span class="ico"><Icon name={ui.error ? "alert" : "check"} size={15} /></span>
    <span
      class="msg"
      role={ui.error ? "alert" : "status"}
      aria-live={ui.error ? "assertive" : "polite"}>{ui.error ?? ui.info}</span
    >
    {#if offerWorkshop && actionable}
      <button class="primary act" onclick={openConflicts}>{t("conflictws.openLong")}</button>
    {/if}
    {#if stashSwitch && actionable}
      <button class="primary act" onclick={() => stashAndSwitch(stashSwitch)}>
        {t("state.stashAndSwitch")}
      </button>
    {/if}
    {#if ui.error && ui.errorAction?.kind === "stashes" && actionable}
      <button class="primary act" onclick={openStashes}>{t("state.openStashes")}</button>
    {/if}
    <button class="ghost" onclick={clearToast} aria-label={t("common.close")}>
      <Icon name="x" size={13} />
    </button>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(720px, calc(100vw - 48px));
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

  .ico {
    display: inline-flex;
    color: var(--accent);
    flex-shrink: 0;
  }

  .toast.error .ico {
    color: var(--deleted);
  }

  .msg {
    user-select: text;
    overflow-wrap: anywhere;
  }

  /* The action button (workshop) must not shrink in the flexing toast. */
  .act {
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>
