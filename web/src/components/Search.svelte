<script lang="ts">
  // Recherche de lieu : autocomplétion Géoplateforme (IGN), ou « lat, lon » saisi à la main.
  import { t } from '../i18n/i18n.svelte';
  import { parseLatLon } from '../lib/geo';

  let { onpick, overlay = false }: { onpick: (p: { lat: number; lon: number; label: string }) => void; overlay?: boolean } = $props();
  let full = $state(false);
  let dlg: HTMLDialogElement | undefined = $state();
  function show(d: HTMLDialogElement) { d.showModal(); const i = d.querySelector('input')!; i.focus(); i.select(); }

  type Hit = { label: string; lat: number; lon: number };
  let q = $state('');
  let hits = $state<Hit[]>([]);
  let active = $state(-1);
  let open = $state(false);
  let searched = $state(false);
  let timer: ReturnType<typeof setTimeout>;
  let ctrl: AbortController | null = null;

  function input() {
    clearTimeout(timer);
    active = -1;
    const ll = parseLatLon(q);
    if (ll) { hits = [{ label: `${ll.lat}, ${ll.lon}`, ...ll }]; open = true; searched = true; return; }
    if (q.trim().length < 3) { hits = []; open = false; searched = false; return; }
    timer = setTimeout(async () => {
      ctrl?.abort();
      ctrl = new AbortController();
      try {
        const u = `https://data.geopf.fr/geocodage/completion?text=${encodeURIComponent(q.trim())}&maximumResponses=6`;
        const r = await (await fetch(u, { signal: ctrl.signal })).json();
        hits = (r.results ?? []).map((x: { fulltext: string; x: number; y: number }) => ({ label: x.fulltext, lat: x.y, lon: x.x }));
      } catch {
        hits = [];
      }
      open = true;
      searched = true;
    }, 250);
  }
  function choose(h: Hit) {
    q = h.label;
    open = false;
    hits = [];
    dlg?.close();
    onpick(h);
  }
  function key(e: KeyboardEvent) {
    if (!open || !hits.length) return;
    if (e.key === 'ArrowDown') { active = (active + 1) % hits.length; e.preventDefault(); }
    else if (e.key === 'ArrowUp') { active = (active - 1 + hits.length) % hits.length; e.preventDefault(); }
    else if (e.key === 'Enter') { choose(hits[Math.max(0, active)]); e.preventDefault(); }
    else if (e.key === 'Escape') { open = false; e.stopPropagation(); }
  }
</script>

{#snippet field()}
  <input
    type="search"
    bind:value={q}
    oninput={input}
    onkeydown={key}
    onblur={() => { if (!overlay) setTimeout(() => (open = false), 150); }}
    placeholder={t().start.search}
    aria-label={t().start.search}
    role="combobox"
    aria-expanded={open}
    aria-controls="search-list"
    aria-autocomplete="list"
    aria-activedescendant={active >= 0 ? `hit-${active}` : undefined}
    autocomplete="off"
    enterkeyhint="search"
  />
{/snippet}
{#snippet list()}
  {#if open}
    <ul id="search-list" role="listbox">
      {#each hits as h, i}
        <li id="hit-{i}" role="option" aria-selected={i === active}>
          <button type="button" tabindex="-1" onmousedown={(e) => e.preventDefault()} onclick={() => choose(h)}>{h.label}</button>
        </li>
      {:else}
        {#if searched}<li class="none">{t().start.noResult}</li>{/if}
      {/each}
    </ul>
  {/if}
{/snippet}

{#if overlay}
  <!-- mobile : la recherche s'ouvre en plein écran, champ en haut (jamais sous le clavier), Retour toujours visible -->
  <button type="button" class="search-open" onclick={() => (full = true)}>
    <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="7" /><path d="M16.500 16.500L21 21" /></svg>
    <span class:ph={!q}>{q || t().start.search}</span>
  </button>
  {#if full}
    <dialog class="fs" bind:this={dlg} use:show onclose={() => (full = false)} aria-label={t().start.search}>
      <div class="fs-head">
        <button type="button" class="icon-btn" onclick={() => dlg?.close()} aria-label={t().result.back} title={t().result.back}>
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.500" /></svg>
        </button>
        <div class="search">{@render field()}</div>
      </div>
      <div class="results">{@render list()}</div>
    </dialog>
  {/if}
{:else}
  <div class="search">
    {@render field()}
    {@render list()}
  </div>
{/if}

<style>
  .search { position: relative; flex: 1; min-width: 0; }
  input { width: 100%; }
  ul {
    position: absolute; left: 0; right: 0; top: calc(100% + 4px); z-index: 20; margin: 0; padding: 4px 0;
    list-style: none; background: var(--surface); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.2); max-height: 50vh; overflow-y: auto;
  }
  li button { all: unset; box-sizing: border-box; display: block; width: 100%; padding: 12px; min-height: 44px; cursor: pointer; }
  li[aria-selected='true'] button, li button:hover { background: var(--accent-soft); }
  .none { padding: 12px; color: var(--muted); }
  .search-open { flex: 1; min-width: 0; min-height: 44px; display: flex; align-items: center; gap: 8px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 10px;
    background: var(--surface); color: var(--text); text-align: left; cursor: pointer; }
  .search-open svg { flex: none; color: var(--muted); }
  .search-open span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ph { color: var(--muted); }
  .results ul { position: static; box-shadow: none; border: 0; background: transparent; max-height: none; margin-top: 8px; }
  .results li + li { border-top: 1px solid var(--border); }
</style>
