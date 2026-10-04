<script lang="ts">
  // Sélecteur d'entraînement sans cartes (D27). Deux variantes (?pick=) :
  //  ridge : une ligne de crêtes, le sommet choisi est le plus haut (plus d'anneaux = plus haut) ;
  //  list  : liste à niveaux, le type choisi se soulève et déplie ses réglages.
  // Même contrat que les cartes v1 : radiogroup, ←/→/↑/↓, nom accessible = nom du type.
  import type { Snippet } from 'svelte';
  import { CATALOG, type TypeId } from '../lib/catalog';
  import { pick } from '../lib/design';
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
  // colline organique (vue de profil), légèrement différente par sommet
  const hill = (i: number) => {
    const s = [0, 6, -4][i % 3];
    return `M-45,64 C-10,62 12,${24 + s} 38,${10 + s} Q50,2 ${60 + s},9 C${80 + s},${22 - s} 105,60 145,64 Z`;
  };
</script>

{#if pick === 'ridge'}
  <div class="ridge" role="radiogroup" aria-label={t().types.label} tabindex="-1" onkeydown={key}>
    {#each types as c, i (c.id)}
      {@const on = value === c.id}
      <button id="type-{c.id}" class="peak" class:on role="radio" aria-checked={on} tabindex={on ? 0 : -1}
        aria-describedby="type-line" onclick={() => onselect(c.id)}>
        <svg viewBox="0 0 100 64" aria-hidden="true">
          <g class="hill">
            <path class="h-fill" d={hill(i)} />
            <path class="h-ring" d={hill(i)} transform="translate(50 64) scale(0.72) translate(-50 -64)" />
            <path class="h-ring r2" d={hill(i)} transform="translate(50 64) scale(0.44) translate(-50 -64)" />
          </g>
          <g transform="translate(38 36)"><path d={c.icon} class="ico" /></g>
        </svg>
        <span class="name">{t().types[c.id].name}</span>
      </button>
    {/each}
  </div>
  <p id="type-line" class="line">{t().types[value].tagline}</p>
  {@render fields()}
{:else}
  <div class="levels" role="radiogroup" aria-label={t().types.label} tabindex="-1" onkeydown={key}>
    {#each types as c (c.id)}
      {@const on = value === c.id}
      <div class="lvl" class:on>
        {#if on}<div class="topo" aria-hidden="true"></div>{/if}
        <button id="type-{c.id}" class="row" role="radio" aria-checked={on} tabindex={on ? 0 : -1} onclick={() => onselect(c.id)}>
          <svg class="blob" viewBox="0 0 24 24" width="36" height="36" aria-hidden="true">
            <path class="b" d="M12 1.5c5 0 10 3 10.4 9.6.4 6.7-5.3 11.6-10.8 11.4C6 22.3 1.4 18.4 1.6 11.8 1.8 5.6 7.3 1.5 12 1.5Z" />
            <g transform="translate(5 5) scale(0.58)"><path d={c.icon} class="ico" /></g>
          </svg>
          <span class="txt"><strong>{t().types[c.id].name}</strong>{#if on}<span>{t().types[c.id].tagline}</span>{/if}</span>
        </button>
        {#if on}<div class="fields">{@render fields()}</div>{/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .ico { fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  /* ---- crêtes ---- */
  .ridge { display: grid; grid-template-columns: repeat(3, 1fr); gap: 0; margin: 4px 0 0; }
  .peak { all: unset; position: relative; display: flex; flex-direction: column; align-items: center; cursor: pointer; min-height: 44px; color: var(--muted); }
  .peak svg { width: 100%; height: 64px; overflow: visible; }
  .peak.on { z-index: 1; color: var(--accent-text); }
  .hill { transform-box: fill-box; transform-origin: 50% 100%; transform: scaleY(0.55); transition: transform 0.45s var(--ease); }
  .on .hill { transform: none; filter: drop-shadow(0 -2px 3px rgb(0 0 0 / 0.18)); }
  .h-fill { fill: var(--lvl-0, var(--surface)); stroke: var(--contour, var(--border)); stroke-width: 1.2; vector-effect: non-scaling-stroke; transition: fill 0.4s; }
  .on .h-fill { fill: var(--accent-soft); stroke: var(--accent); }
  .h-ring { fill: none; stroke: var(--contour, var(--border)); stroke-width: 1; vector-effect: non-scaling-stroke; opacity: 0.55; }
  .r2 { opacity: 0; transition: opacity 0.4s; }
  .on .h-ring { stroke: var(--accent); }
  .on .r2 { opacity: 0.55; }
  .name { font-weight: 700; margin-top: 2px; color: var(--text); text-align: center; line-height: 1.2; }
  .peak.on .name { text-decoration: underline; text-decoration-thickness: 2px; text-underline-offset: 4px; text-decoration-color: var(--accent); }
  .peak:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; border-radius: 8px; }
  .line { margin: 6px 0 4px; color: var(--muted); text-align: center; font-size: 0.95rem; }
  /* ---- niveaux ---- */
  .levels { display: flex; flex-direction: column; gap: 6px; margin: 4px 0 8px; }
  .lvl { position: relative; border-radius: 18px 22px 16px 24px / 20px 16px 24px 18px; transition: background-color 0.35s, box-shadow 0.35s var(--ease); overflow: hidden; }
  .lvl.on { background: var(--lvl-3, var(--bg)); box-shadow: 0 1px 2px rgb(0 0 0 / 0.12), 0 6px 18px rgb(0 0 0 / 0.14); padding-bottom: 4px; }
  .lvl .topo { height: 70px; inset: 0 0 auto 0; -webkit-mask-image: var(--topo), linear-gradient(#000, transparent); mask-image: var(--topo), linear-gradient(#000, transparent);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: 480px, 100% 100%; mask-size: 480px, 100% 100%; }
  .row { all: unset; position: relative; box-sizing: border-box; width: 100%; display: flex; gap: 12px; align-items: center; min-height: 48px; padding: 6px 12px; cursor: pointer; }
  .row:focus-visible { outline: 3px solid var(--accent); outline-offset: -3px; border-radius: 16px; }
  .blob { flex: none; color: var(--muted); }
  .blob .b { fill: var(--surface); stroke: var(--contour, var(--border)); stroke-width: 1; transition: fill 0.3s; }
  .on .blob { color: var(--on-accent); }
  .on .blob .b { fill: var(--accent); stroke: none; }
  .txt { display: flex; flex-direction: column; }
  .txt strong { font-size: 1.05rem; }
  .txt span { color: var(--muted); font-size: 0.92rem; line-height: 1.25; }
  .fields { position: relative; padding: 0 12px; }
  @media (prefers-reduced-motion: reduce) { .hill, .r2, .h-fill, .lvl { transition: none; } }
</style>
