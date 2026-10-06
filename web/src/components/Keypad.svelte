<script lang="ts">
  // Pavé numérique intégré à la feuille (mobile) : pas le clavier du système, donc rien ne bouge, hauteur connue,
  // même comportement sur iOS et Android, chiffres seulement. Geste : toucher la valeur → 2 ou 3 chiffres → OK, ou une
  // puce. Le premier chiffre remplace la valeur affichée. −/+ : ajustement fin. Clavier physique : chiffres, virgule,
  // Retour arrière, Entrée ; Échap et le bouton Retour du téléphone annulent (couche `pad`).
  import { untrack } from 'svelte';
  import { bump, padKey, padParse, type Field } from '../lib/catalog';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num } from '../i18n/format';

  let { field, value, auto, minHeight, oncommit, oncancel }: {
    field: Field; value: number | null; auto: number | null; minHeight: number; oncommit: (v: number | null) => void; oncancel: () => void;
  } = $props();

  const str = (x: number) => String(x).replace('.', ',');
  const fmt = (x: number) => `${num(i18n.lang, x, 1)} ${field.unit}`;
  const decimals = $derived(field.step < 1);
  const name = $derived(t().fields[field.param]);
  // valeur de départ seulement (le pavé est recréé à chaque ouverture)
  const v0 = untrack(() => value);
  let raw = $state(v0 == null ? '' : str(v0));
  let fresh = $state(true);
  const parsed = $derived(padParse(raw, field));

  function press(k: string) {
    raw = padKey(fresh && k !== ',' ? '' : raw, k, decimals);
    fresh = false;
  }
  function nudge(dir: 1 | -1) {
    raw = str(bump(field, parsed.v ?? auto ?? field.def ?? field.min, dir));
    fresh = true;
  }
  // focus sur le groupe (pas un champ : le clavier du système ne s'ouvre pas) pour la saisie au clavier physique
  function focus(el: HTMLElement) { el.focus({ preventScroll: true }); }
  function ok() { if (parsed.ok) oncommit(parsed.v); }
  function key(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (/^\d$/.test(e.key)) press(e.key);
    else if (e.key === ',' || e.key === '.') press(',');
    else if (e.key === 'Backspace') press('back');
    else if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'BUTTON') ok();
    else return;
    e.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="pad" style:min-height="{minHeight}px" role="group" aria-labelledby="pad-name" tabindex="-1" onkeydown={key} use:focus>
  <div class="top">
    <div class="who">
      <strong id="pad-name">{name}</strong>
      <span id="pad-range" class="range" class:bad={!parsed.ok && raw !== ''}>{t().fields.range({ min: fmt(field.min), max: fmt(field.max) })}</span>
    </div>
    <button type="button" class="pm" onclick={() => nudge(-1)} disabled={(parsed.v ?? field.min + 1) <= field.min} aria-label={t().fields.less({ name })}>
      <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M5 12h14" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" /></svg>
    </button>
    <output id="pad-value" class="value" class:fresh aria-live="polite" aria-describedby="pad-range">
      {#if raw}<strong>{raw}</strong>{:else if auto != null}<small>auto</small>{:else}<strong>–</strong>{/if}
      <span class="unit">{field.unit}</span>
    </output>
    <button type="button" class="pm" onclick={() => nudge(1)} disabled={(parsed.v ?? field.min) >= field.max} aria-label={t().fields.more({ name })}>
      <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M5 12h14M12 5v14" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" /></svg>
    </button>
  </div>
  <div class="chips" role="group" aria-label={t().fields.presets}>
    {#if field.optional}
      <button type="button" class="chip" aria-pressed={value == null} onclick={() => oncommit(null)}>{t().fields.autoToggle}</button>
    {/if}
    {#each field.presets as p}
      <button type="button" class="chip" aria-pressed={value === p} aria-label={fmt(p)} onclick={() => oncommit(p)}>{num(i18n.lang, p)}</button>
    {/each}
  </div>
  <div class="keys">
    {#each ['1', '2', '3', 'back', '4', '5', '6', decimals ? ',' : '00', '7', '8', '9'] as k}
      {#if k === 'back'}
        <button type="button" class="k fn" data-k="back" onclick={() => press('back')} aria-label={t().fields.erase}>
          <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 5h11v14H9l-6-7z" /><path d="M12 9.500l5 5M17 9.500l-5 5" /></svg>
        </button>
      {:else}
        <button type="button" class="k" class:fn={k === ',' || k === '00'} data-k={k} onclick={() => press(k)}>{k}</button>
      {/if}
    {/each}
    <button type="button" class="k ok" onclick={ok} disabled={!parsed.ok}>{t().important.ok}</button>
    <button type="button" class="k fn cancel" onclick={oncancel}>{t().compute.cancel}</button>
    <button type="button" class="k zero" data-k="0" onclick={() => press('0')}>0</button>
  </div>
</div>

<style>
  .pad { display: flex; flex-direction: column; gap: 8px; padding-bottom: 12px; outline: none; }
  .top { display: flex; align-items: center; gap: 6px; min-height: 48px; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; line-height: 1.25; }
  .range { color: var(--muted); font-size: 0.85rem; }
  .range.bad { color: var(--danger); font-weight: 700; }
  .pm { flex: none; width: 44px; height: 44px; border-radius: 50%; border: 1.5px solid var(--border); background: var(--surface); color: var(--accent-text);
    display: grid; place-items: center; padding: 0; cursor: pointer; }
  .pm:disabled { opacity: 0.4; cursor: not-allowed; }
  .value { flex: none; min-width: 5.2em; text-align: center; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .value strong { font-size: 1.7rem; line-height: 1; border-bottom: 2px solid var(--accent); }
  /* valeur de départ : le premier chiffre la remplace (surlignée comme une sélection) */
  .value.fresh strong { background: var(--accent-soft); border-radius: 4px 4px 0 0; padding: 0 3px; }
  .value small { color: var(--muted); font-size: 1.1rem; }
  .unit { color: var(--muted); font-weight: 600; }
  .chips { display: flex; gap: 6px; overflow-x: auto; margin: 0 -16px; padding: 0 16px; scrollbar-width: none; }
  .chips::-webkit-scrollbar { display: none; }
  .chips .chip { flex: 1 0 auto; min-width: 48px; padding: 6px 10px; font-variant-numeric: tabular-nums; }
  .keys { flex: 1; display: grid; grid-template-columns: repeat(4, 1fr); grid-auto-rows: minmax(44px, 1fr); gap: 6px; }
  .k { border: 1.5px solid var(--border); border-radius: 10px; background: var(--bg); color: var(--text); font-size: 1.35rem; font-weight: 600; cursor: pointer; padding: 0;
    display: grid; place-items: center; touch-action: manipulation; -webkit-user-select: none; user-select: none; }
  .k:active:not(:disabled) { background: var(--accent-soft); }
  .k.fn { background: var(--surface); }
  .k.cancel { font-size: 0.95rem; }
  .k.zero { grid-column: span 2; }
  .k.ok { grid-row: span 2; background: var(--accent); border-color: var(--accent); color: var(--on-accent); font-weight: 800; font-size: 1.15rem; }
  .k.ok:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
