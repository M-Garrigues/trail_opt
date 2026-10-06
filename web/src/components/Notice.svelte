<script lang="ts">
  // « Messages importants » (D40) : popup modale sobre quand la demande n'est pas atteinte ; action proposée facultative.
  import { app, dismissNotice, applySuggest, suggestLabel } from '../lib/app.svelte';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { warningText } from '../i18n/format';
  import { onMount } from 'svelte';

  const n = $derived(app.notice!);
  const lines = $derived(n.warnings.map((w) => warningText(i18n.lang, w.code, w.params ?? {})).filter(Boolean));
  // action : premier `suggest` du serveur (clé d'API → libellé du front, jamais de phrase du moteur)
  const sg = $derived(n.warnings.find((w) => w.suggest && Object.keys(w.suggest).length)?.suggest ?? null);
  const label = $derived(suggestLabel(sg));
  let dlg: HTMLElement;
  onMount(() => dlg.querySelector<HTMLButtonElement>('button')?.focus());
</script>

<div class="scrim">
  <div class="dlg" bind:this={dlg} role="alertdialog" aria-modal="true" aria-labelledby="imp-t" aria-describedby="imp-d">
    <h2 id="imp-t">{t().important.title}</h2>
    <div id="imp-d">
      {#each lines as l}<p>{l}</p>{/each}
    </div>
    <div class="row">
      {#if sg && label}<button class="btn primary" onclick={() => applySuggest(sg)}>{label}</button>{/if}
      <button class="btn" class:secondary={sg && label} class:primary={!(sg && label)} onclick={dismissNotice}>{sg && label ? t().important.keep : t().important.ok}</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 30; background: rgb(0 0 0 / 0.4); display: grid; place-items: center; padding: 16px; }
  .dlg { background: var(--lvl-3); color: var(--text); border-radius: 14px; padding: 18px; max-width: 380px; width: 100%;
    box-shadow: 0 2px 4px rgb(var(--sh-rgb) / 0.2), 0 12px 40px rgb(var(--sh-rgb) / 0.3); }
  h2 { font-size: 1.15rem; margin: 0 0 8px; }
  p { margin: 0 0 8px; }
  .row { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 12px; }
  .row .btn { flex: 1; }
</style>
