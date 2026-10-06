<script lang="ts">
  // Sélecteur d'entraînement en liste (D27, D34 : extensible, fondu dans le panneau). Le type choisi « monte » :
  // il se soulève (ombre en couches, teinte plus claire), ses courbes de niveau glissent en place et ses réglages
  // se déplient dessous. Contrat a11y : radiogroup, ←/→/↑/↓, nom accessible = nom du type.
  import type { Snippet } from 'svelte';
  import { CATALOG, type TypeId } from '../lib/catalog';
  import { t } from '../i18n/i18n.svelte';

  let { value, onselect, fields }: { value: TypeId; onselect: (id: TypeId) => void; fields: Snippet } = $props();
  const types = CATALOG.filter((c) => c.enabled);

  function key(e: KeyboardEvent) {
    if ((e.target as HTMLElement).getAttribute('role') !== 'radio') return;
    const i = types.findIndex((c) => c.id === value);
    let j = i;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') j = (i + 1) % types.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') j = (i - 1 + types.length) % types.length;
    else return;
    e.preventDefault();
    onselect(types[j].id);
    document.getElementById(`type-${types[j].id}`)?.focus();
  }
</script>

<div class="levels" role="radiogroup" aria-label={t().types.label} tabindex="-1" onkeydown={key}>
  {#each types as c (c.id)}
    {@const on = value === c.id}
    <div class="lvl" class:on>
      <div class="topo t-{c.id}" aria-hidden="true"></div>
      <button id="type-{c.id}" class="row" role="radio" aria-checked={on} tabindex={on ? 0 : -1} onclick={() => onselect(c.id)}>
        <span class="ico-wrap" aria-hidden="true">
          <svg viewBox="0 0 24 24" width="22" height="22"><path d={c.icon} class="ico" /></svg>
        </span>
        <span class="txt"><strong>{t().types[c.id].name}</strong><span>{t().types[c.id].tagline}</span></span>
      </button>
      {#if on}<div class="fields">{@render fields()}</div>{/if}
    </div>
  {/each}
</div>

<style>
  .levels { display: flex; flex-direction: column; gap: 2px; margin: 4px -8px 8px; }
  .lvl { position: relative; border-radius: 12px; overflow: hidden;
    transition: background-color 0.35s, box-shadow 0.45s var(--ease), transform 0.45s var(--ease); }
  .lvl:not(.on) + .lvl:not(.on)::before { content: ''; position: absolute; top: -1px; left: 60px; right: 12px; border-top: 1px solid var(--border); opacity: 0.6; }
  .lvl:not(.on) { background: var(--surface); box-shadow: inset 0 0 0 1px var(--border); }
  .lvl.on { background: var(--lvl-3); box-shadow: var(--sh-2); transform: translateY(-2px); margin: 4px 0; }
  /* courbes de niveau sur le côté de chaque carte (cliquable) ; elles s'étendent sur toute la carte choisie */
  .lvl .topo { inset: 0 0 0 auto; width: 56px; transition: width 0.5s var(--ease);
    -webkit-mask-image: var(--topo), linear-gradient(to left, #000 0, transparent 100%); mask-image: var(--topo), linear-gradient(to left, #000 0, transparent 100%);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: var(--ts, 1000px), 100% 100%; mask-size: var(--ts, 1000px), 100% 100%;
    -webkit-mask-position: var(--tp, -420px -560px), 0 0; mask-position: var(--tp, -420px -560px), 0 0; }
  /* un motif fixe par type : zones de la carte de courbes choisies à densité d'encre comparable (~20-25 %), échelles différentes */
  .t-max_dplus { --ts: 700px; --tp: -70px 0; }
  .t-target { --ts: 900px; --tp: 0 -450px; }
  .t-shortest { --ts: 1000px; --tp: 0 -300px; }
  .lvl.on .topo { width: 100%; }
  .row { all: unset; position: relative; box-sizing: border-box; width: 100%; display: flex; gap: 12px; align-items: center; min-height: 52px; padding: 6px 12px; cursor: pointer; }
  .row:focus-visible { outline: 3px solid var(--accent); outline-offset: -3px; border-radius: 12px; }
  .lvl:not(.on) .row:hover { background: var(--lvl-2); border-radius: 12px; }
  .ico-wrap { flex: none; width: 36px; height: 36px; display: grid; place-items: center; border-radius: 50%; color: var(--muted);
    background: var(--lvl-2); transition: background-color 0.3s, color 0.3s; }
  .on .ico-wrap { background: var(--accent); color: var(--on-accent); }
  .ico { fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .txt { display: flex; flex-direction: column; }
  .txt strong { font-size: 1.05rem; }
  .txt span { color: var(--muted); font-size: 0.92rem; line-height: 1.25; }
  .fields { position: relative; padding: 0 12px 4px; }
  @media (prefers-reduced-motion: reduce) { .lvl, .ico-wrap { transition: none; } .lvl .topo { transition: none; } }
</style>
