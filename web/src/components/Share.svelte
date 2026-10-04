<script lang="ts">
  // E9 partage : confirmation (le lien montre le départ, 90 j) avant l'appel POST /api/loops.
  import { app, close, shareLoop } from '../lib/app.svelte';
  import { t } from '../i18n/i18n.svelte';

  let url = $state<string | null>(null);
</script>

<div class="head">
  <button class="icon-btn" onclick={close} aria-label={t().result.back}>
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
  </button>
  <h2>{t().result.share}</h2>
</div>
<p>{t().share.notice}</p>
<button class="btn primary wide" disabled={app.sharing || app.offline} onclick={async () => (url = (await shareLoop()) ?? url)}>
  {app.sharing ? t().share.creating : t().share.create}
</button>
{#if url}
  <p class="url"><input readonly value={url} aria-label={t().share.create} onfocus={(e) => e.currentTarget.select()} /></p>
{/if}

<style>
  .head { display: flex; gap: 8px; align-items: center; }
  h2 { font-size: 1.25rem; margin: 0; }
  .url input { width: 100%; min-height: 44px; padding: 8px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); color: var(--text); }
</style>
