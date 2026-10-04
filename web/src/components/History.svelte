<script lang="ts">
  // E12 « Mes boucles » : historique local (D24), plus récentes d'abord, « Effacer l'historique » toujours visible.
  import { close, openEntry, toast } from '../lib/app.svelte';
  import * as hist from '../lib/history';
  import { byId } from '../lib/catalog';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, km } from '../i18n/format';

  let entries = $state(hist.list());
  const L = $derived(i18n.lang);
  const date = (ts: number) => new Intl.DateTimeFormat(L, { dateStyle: 'medium', timeStyle: 'short' }).format(ts);

  function clearAll() {
    if (!confirm(t().history.confirmClear)) return;
    hist.clear();
    entries = hist.list();
  }
  function del(id: string) {
    if (hist.removeEntry(id) !== 'ok') toast(t().history.unavailable);
    entries = hist.list();
  }
</script>

<div class="head">
  <button class="icon-btn" onclick={close} aria-label={t().result.back}>
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
  </button>
  <h2>{t().history.title}</h2>
  <button class="btn danger small" onclick={clearAll} disabled={!entries.length}>{t().history.clear}</button>
</div>
{#if !entries.length}
  <p class="muted">{t().history.empty}</p>
{:else}
  <ul class="list" data-testid="history">
    {#each entries as e (e.id)}
      <li>
        <button class="open" onclick={() => openEntry(e)}>
          <strong>{t().types[byId(e.settings.typeId).id].name} · {km(L, e.candidate.length_m)} · +{num(L, e.candidate.dplus_m)} m</strong>
          <span>{date(e.ts)}</span>
        </button>
        <button class="icon-btn" onclick={() => del(e.id)} aria-label="{t().history.remove} : {km(L, e.candidate.length_m)}, {date(e.ts)}">✕</button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .head { display: flex; gap: 8px; align-items: center; position: sticky; top: 0; background: var(--bg); padding: 4px 0; z-index: 1; }
  h2 { font-size: 1.25rem; margin: 0; flex: 1; }
  .list { list-style: none; padding: 0; margin: 8px 0; }
  .list li { display: flex; align-items: center; gap: 4px; border-bottom: 1px solid var(--border); }
  .open { all: unset; box-sizing: border-box; flex: 1; padding: 10px 4px; cursor: pointer; display: flex; flex-direction: column; min-height: 44px; }
  .open:focus-visible { outline: 3px solid var(--accent); }
  .open span { color: var(--muted); font-size: 0.875rem; }
  .muted { color: var(--muted); }
</style>
