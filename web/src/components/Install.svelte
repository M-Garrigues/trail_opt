<script lang="ts">
  // Installer l'app : entrée permanente du menu (défaut) ou suggestion unique, refermable, hors calcul (`hint`).
  import { has } from '../lib/app.svelte';
  import { install, installHow, promptInstall, dismissHint } from '../lib/install.svelte';
  import { t } from '../i18n/i18n.svelte';

  let { hint = false }: { hint?: boolean } = $props();
  const h = $derived(installHow());
  const steps = $derived(t().install[h === 'ios' ? 'ios' : 'manual']);
  let open = $state(false);
</script>

{#snippet share()}
  <!-- pictogramme du bouton Partager d'iOS -->
  <svg class="share" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 15V3M8 7l4-4 4 4M8 11H6v10h12V11h-2" />
  </svg>
{/snippet}

{#if h === 'none'}
  <!-- déjà installée, ou navigateur sans installation : rien -->
{:else if hint}
  {#if install.hint && !has('computing')}
    <section class="hint" aria-label={t().install.title} data-testid="install-hint">
      <div class="txt" role="status">
        <p><strong>{t().install.hint}</strong></p>
        {#if h !== 'prompt'}<p class="steps">{#if h === 'ios'}{@render share()}{/if}{steps}</p>{/if}
      </div>
      {#if h === 'prompt'}<button class="btn primary small" onclick={promptInstall}>{t().install.action}</button>{/if}
      <button class="icon-btn" onclick={dismissHint} aria-label={t().install.later} title={t().install.later}>✕</button>
    </section>
  {/if}
{:else if h === 'prompt'}
  <button class="btn secondary wide entry" onclick={promptInstall}>{t().install.title}</button>
{:else}
  <button class="btn secondary wide entry" aria-expanded={open} aria-controls="install-steps" onclick={() => (open = !open)}>{t().install.title}</button>
  <p id="install-steps" class="steps box" hidden={!open}>{#if h === 'ios'}{@render share()}{/if}{steps}</p>
{/if}

<style>
  .entry { margin-top: 8px; }
  .steps { margin: 0; line-height: 1.45; }
  .steps.box { margin-top: 8px; padding: 10px 12px; border-radius: 10px; background: var(--surface); border: 1px solid var(--border); }
  .share { vertical-align: -4px; margin-right: 6px; color: var(--accent-text); }
  /* même emplacement que les messages (sous la barre du haut), sous la feuille et à gauche du bouton 3D */
  .hint {
    position: absolute; top: calc(64px + env(safe-area-inset-top)); left: 8px; right: 60px; z-index: 4; display: flex; gap: 8px; align-items: center;
    background: var(--lvl-3); color: var(--text); border: 1px solid var(--border); border-radius: 12px; padding: 10px 8px 10px 12px;
    box-shadow: var(--sh-2); animation: in 0.3s var(--ease);
  }
  :global(.app.desktop) .hint { left: 416px; right: auto; width: 420px; }
  .txt { flex: 1; min-width: 0; }
  .txt p { margin: 0; }
  .txt .steps { margin-top: 4px; }
  .hint .icon-btn { align-self: flex-start; border-color: transparent; background: transparent; }
  @keyframes in { from { opacity: 0; transform: translateY(-8px); } }
  @media (prefers-reduced-motion: reduce) { .hint { animation: none; } }
</style>
