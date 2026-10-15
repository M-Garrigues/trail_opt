<script lang="ts">
  // Sélecteur de sport (D70) : radiogroup segmenté, ←/→ ; pictogramme seul sur mobile, pictogramme + nom sur ordinateur.
  // Bouton choisi plein (accent du sport) ; désactivé pendant un calcul, l'édition de zone et la saisie d'une valeur.
  import { SPORTS } from '../lib/catalog';
  import { sport } from '../lib/sport.svelte';
  import { setSport, sportLocked } from '../lib/app.svelte';
  import { SPORT_PATH } from '../lib/icons';
  import { t } from '../i18n/i18n.svelte';

  let { labels = false }: { labels?: boolean } = $props();
  const locked = $derived(sportLocked());

  function key(e: KeyboardEvent) {
    const i = SPORTS.indexOf(sport.id);
    let j = i;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') j = (i + 1) % SPORTS.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') j = (i - 1 + SPORTS.length) % SPORTS.length;
    else return;
    e.preventDefault();
    void setSport(SPORTS[j]);
    document.getElementById(`sport-${SPORTS[j]}`)?.focus();
  }
</script>

<div class="sports" role="radiogroup" aria-label={t().sport.label} tabindex="-1" onkeydown={key}>
  {#each SPORTS as s (s)}
    {@const on = sport.id === s}
    <button id="sport-{s}" type="button" role="radio" aria-checked={on} tabindex={on ? 0 : -1} disabled={locked}
      aria-label={t().sport[s]} title={t().sport[s]} onclick={() => setSport(s)}>
      <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d={SPORT_PATH[s]} /></svg>
      {#if labels}<span>{t().sport[s]}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .sports { pointer-events: auto; display: flex; background: var(--bg); border-radius: 10px; box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); padding: 2px; gap: 2px; margin-right: auto; }
  button { all: unset; box-sizing: border-box; min-width: 44px; min-height: 44px; display: inline-flex; align-items: center; justify-content: center; gap: 6px;
    padding: 0 10px; border-radius: 8px; cursor: pointer; color: var(--muted); font-weight: 600; }
  button[aria-checked='true'] { background: var(--accent); color: var(--on-accent); }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  button:focus-visible { outline: 3px solid var(--text); outline-offset: -3px; }
</style>
