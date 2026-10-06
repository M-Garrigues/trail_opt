<script lang="ts">
  // Feuille du bas (mobile) ; bureau : panneau latéral.
  // Deux états seulement, sans poignée ni glissement : dépliée (`fit` = hauteur du contenu ; sinon `full` px ou 90 %) ou
  // repliée sur UNE ligne de résumé. `summary` non vide = feuille repliable : chevron d'accent en haut à droite
  // (replié : toute la ligne de résumé est le bouton). État gardé en mémoire le temps de la session (app.snap : 0 / 2).
  // Aucune gestion du clavier ici : la saisie se fait au pavé intégré ou dans un plein écran (<dialog class="fs">).
  // Niveaux (D27, D34) : dépliée, la feuille « monte » : ombre en couches plus large, teinte plus claire.
  import type { Snippet } from 'svelte';
  import { app, level as uiLevel } from '../lib/app.svelte';
  import { t } from '../i18n/i18n.svelte';

  let { desktop, children, label, summary = '', fit = false, full = 0 }: { desktop: boolean; children: Snippet; label: string; summary?: string; fit?: boolean; full?: number } = $props();

  const level = $derived(uiLevel(desktop));
  const folded = $derived(!!summary && app.snap === 0);
  let content: HTMLElement | undefined = $state();
  // à chaque changement d'état/de couche : contenu tout en haut
  $effect(() => {
    void app.snap; void app.layers.length;
    if (content) content.scrollTop = 0;
  });
</script>

{#if desktop}
  <aside class="panel" aria-label={label} data-level={level}>
    <div class="topo head-topo"></div>
    <div class="scroll">{@render children()}</div>
  </aside>
{:else}
  <section class="sheet" class:fit class:folded class:foldable={!!summary} aria-label={label} data-level={folded ? 1 : level}>
    <div class="topo head-topo"></div>
    {#if summary}
      <button class="fold" aria-expanded={!folded} aria-controls="sheet-content" aria-label={folded ? `${t().sheet.unfold} : ${summary}` : t().sheet.fold}
        onclick={() => (app.snap = folded ? 2 : 0)}>
        {#if folded}<span class="sum">{summary}</span>{/if}
        <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true"><path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2.800" stroke-linecap="round" stroke-linejoin="round" /></svg>
      </button>
    {/if}
    <div class="body"><div class="clip">
      <div class="content" id="sheet-content" bind:this={content} style:height={fit ? 'auto' : `${full || Math.round(innerHeight * 0.9)}px`}>
        {@render children()}
      </div>
    </div></div>
  </section>
{/if}

<style>
  .panel, .sheet { position: absolute; z-index: 5; color: var(--text); background: var(--lvl-2); box-sizing: border-box; display: flex; flex-direction: column;
    transition: box-shadow 0.4s var(--ease), background-color 0.4s; }
  .panel { inset: 0 auto 0 0; width: 400px; }
  .scroll { position: relative; flex: 1; overflow-y: auto; padding: 12px 16px 0; display: flex; flex-direction: column; }
  .scroll > :global(*) { flex: none; }
  .sheet { left: 0; right: 0; bottom: 0; border-radius: 14px 14px 0 0; padding-bottom: env(safe-area-inset-bottom); }
  /* dépliage / repliage : la rangée de grille passe de 1fr à 0fr (hauteur animée sans mesure) */
  .body { display: grid; grid-template-rows: 1fr; transition: grid-template-rows 0.22s var(--ease); min-height: 0; }
  .folded .body { grid-template-rows: 0fr; }
  .clip { min-height: 0; overflow: hidden; }
  .content { position: relative; min-height: 0; max-height: 90vh; max-height: calc(90dvh - env(safe-area-inset-bottom)); overflow-y: auto; padding: 12px 16px 0; overscroll-behavior: contain;
    display: flex; flex-direction: column; box-sizing: border-box; }
  .content > :global(*) { flex: none; }
  /* repliée : RIEN du contenu ne reste visible ni atteignable */
  .folded .content { visibility: hidden; overflow: hidden; padding-top: 0; transition: visibility 0s 0.22s; }
  /* la première ligne du contenu laisse la place du chevron */
  .foldable .content > :global(:first-child) { margin-right: 48px; }
  .fold { position: absolute; z-index: 2; top: 12px; right: 12px; width: 44px; height: 44px; padding: 0; border: 0; border-radius: 10px; background: transparent;
    color: var(--accent-text); cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .fold svg { flex: none; transition: transform 0.22s var(--ease); }
  .folded .fold { position: relative; top: 0; right: 0; width: 100%; height: auto; min-height: 56px; padding: 0 21px 0 16px; gap: 8px; justify-content: space-between; text-align: left; }
  .folded .fold svg { transform: rotate(180deg); }
  .sum { min-width: 0; font-weight: 700; font-size: 1.05rem; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-variant-numeric: tabular-nums; }
  .head-topo { inset: 0 0 auto 0; height: 140px; max-height: 100%; border-radius: inherit;
    -webkit-mask-image: var(--topo), linear-gradient(#000, transparent); mask-image: var(--topo), linear-gradient(#000, transparent);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: 1000px, 100% 100%; mask-size: 1000px, 100% 100%; }
  /* élévation : ombre vers la carte (haut pour la feuille, droite pour le panneau) */
  .sheet[data-level='1'] { --here: var(--lvl-1); background: var(--lvl-1); box-shadow: 0 -1px 1px rgb(var(--sh-rgb) / 0.10), 0 -2px 6px rgb(var(--sh-rgb) / 0.10); }
  .sheet[data-level='2'] { --here: var(--lvl-2); background: var(--lvl-2); box-shadow: 0 -1px 2px rgb(var(--sh-rgb) / 0.12), 0 -4px 10px rgb(var(--sh-rgb) / 0.12), 0 -12px 28px rgb(var(--sh-rgb) / 0.12); }
  .sheet[data-level='3'] { --here: var(--lvl-3); background: var(--lvl-3); box-shadow: 0 -1px 2px rgb(var(--sh-rgb) / 0.14), 0 -6px 14px rgb(var(--sh-rgb) / 0.14), 0 -20px 44px rgb(var(--sh-rgb) / 0.16); }
  .panel[data-level='1'] { --here: var(--lvl-1); background: var(--lvl-1); box-shadow: 1px 0 1px rgb(var(--sh-rgb) / 0.10), 2px 0 6px rgb(var(--sh-rgb) / 0.10); }
  .panel[data-level='2'] { --here: var(--lvl-2); background: var(--lvl-2); box-shadow: 1px 0 2px rgb(var(--sh-rgb) / 0.12), 4px 0 10px rgb(var(--sh-rgb) / 0.12), 12px 0 28px rgb(var(--sh-rgb) / 0.12); }
  .panel[data-level='3'] { --here: var(--lvl-3); background: var(--lvl-3); box-shadow: 1px 0 2px rgb(var(--sh-rgb) / 0.14), 6px 0 14px rgb(var(--sh-rgb) / 0.14), 20px 0 44px rgb(var(--sh-rgb) / 0.16); }
  @media (prefers-reduced-motion: reduce) { .sheet, .panel, .body, .fold svg, .folded .content { transition: none; } }
</style>
