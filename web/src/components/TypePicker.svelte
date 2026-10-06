<script lang="ts">
  // Sélecteur du type de sortie (D27, D34 : extensible). Contrat a11y : radiogroup, ←/→/↑/↓, nom accessible = nom du type.
  // Intitulé de groupe visible. Mobile (`compact`) : rangée de puces (Material 3 « filter chips ») sur UNE ligne, qui
  // défile à l'horizontale quand les types ne tiennent plus (la puce coupée au bord amorce la suite) : 3 ou 7 types, même
  // hauteur. La puce choisie est PLEINE (accent) et cochée, toujours ramenée en vue ; sa description et ses réglages sont
  // juste dessous. Bureau : liste à boutons radio (pastille cochée + liseré accent), réglages sous l'option choisie.
  import type { Snippet } from 'svelte';
  import { CATALOG, type TypeId } from '../lib/catalog';
  import { t } from '../i18n/i18n.svelte';

  let { value, onselect, fields, compact = false, aside }: { value: TypeId; onselect: (id: TypeId) => void; fields: Snippet; compact?: boolean; aside?: Snippet } = $props();
  const types = CATALOG.filter((c) => c.enabled);
  // la puce choisie reste visible (type mémorisé en fin de rangée, flèches du clavier)
  $effect(() => { document.getElementById(`type-${value}`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' }); });

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

<p class="group" id="type-l">{t().types.label}</p>
{#if compact}
  <div class="seg" role="radiogroup" aria-labelledby="type-l" tabindex="-1" onkeydown={key}>
    {#each types as c (c.id)}
      {@const on = value === c.id}
      <button id="type-{c.id}" type="button" role="radio" aria-checked={on} tabindex={on ? 0 : -1} onclick={() => onselect(c.id)}>
        {#if on}<svg class="check" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M5 12.500l4.500 4.500L19 7.500" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" /></svg>
        {:else}<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d={c.icon} class="ico" /></svg>{/if}
        {t().types[c.id].name}
      </button>
    {/each}
  </div>
  <p class="tag"><span>{t().types[value].tagline}</span>{@render aside?.()}</p>
  {@render fields()}
{:else}
  <div class="levels" role="radiogroup" aria-labelledby="type-l" tabindex="-1" onkeydown={key}>
    {#each types as c (c.id)}
      {@const on = value === c.id}
      <div class="lvl" class:on>
        <div class="topo t-{c.id}" aria-hidden="true"></div>
        <button id="type-{c.id}" type="button" class="row" role="radio" aria-checked={on} tabindex={on ? 0 : -1} onclick={() => onselect(c.id)}>
          <span class="ico-wrap" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="22" height="22"><path d={c.icon} class="ico" /></svg>
          </span>
          <span class="txt"><strong>{t().types[c.id].name}</strong><span>{t().types[c.id].tagline}</span></span>
          <span class="radio" aria-hidden="true"></span>
        </button>
        {#if on}<div class="fields">{@render fields()}</div>{/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .group { margin: 0 0 4px; font-size: 0.8rem; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); }
  /* mobile : rangée de puces défilante (jusqu'au bord de l'écran) */
  .seg { display: flex; gap: 6px; overflow-x: auto; margin: 0 -16px; padding: 2px 16px; scroll-padding-inline: 16px; scrollbar-width: none; overscroll-behavior-x: contain; }
  .seg::-webkit-scrollbar { display: none; }
  .seg button { all: unset; box-sizing: border-box; flex: 1 0 auto; min-height: 46px; padding: 4px 12px; display: flex; align-items: center; justify-content: center; gap: 6px;
    white-space: nowrap; font-weight: 600; font-size: 0.95rem; cursor: pointer; border: 1.5px solid var(--border); border-radius: 23px; background: var(--surface); }
  .seg button svg { flex: none; color: var(--muted); }
  .seg button[aria-checked='true'] { background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 800; }
  .seg button[aria-checked='true'] svg { color: inherit; }
  .seg button:focus-visible { outline: 3px solid var(--text); outline-offset: -4px; }
  .tag { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; margin: 6px 0 8px; color: var(--muted); font-size: 0.92rem; line-height: 1.25; }
  /* bureau : liste à boutons radio */
  .levels { display: flex; flex-direction: column; gap: 2px; margin: 4px -8px 8px; }
  .lvl { position: relative; border-radius: 12px; overflow: hidden;
    transition: background-color 0.35s, box-shadow 0.45s var(--ease), transform 0.45s var(--ease); }
  .lvl:not(.on) { background: var(--surface); box-shadow: inset 0 0 0 1px var(--border); }
  .lvl.on { background: var(--lvl-3); box-shadow: inset 0 0 0 2px var(--accent), var(--sh-2); transform: translateY(-2px); margin: 4px 0; }
  /* courbes de niveau sur le côté de chaque carte (décor) ; elles s'étendent sur toute la carte choisie */
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
  .txt { display: flex; flex-direction: column; flex: 1; }
  .txt strong { font-size: 1.05rem; }
  .txt span { color: var(--muted); font-size: 0.92rem; line-height: 1.25; }
  /* pastille radio : vide = à choisir, pleine = choisie */
  .radio { flex: none; width: 22px; height: 22px; border-radius: 50%; border: 2px solid var(--muted); background: var(--bg); display: grid; place-items: center; }
  .on .radio { border-color: var(--accent); }
  .on .radio::after { content: ''; width: 12px; height: 12px; border-radius: 50%; background: var(--accent); }
  .fields { position: relative; padding: 0 12px 4px; }
  @media (prefers-reduced-motion: reduce) { .lvl, .ico-wrap { transition: none; } .lvl .topo { transition: none; } }
</style>
