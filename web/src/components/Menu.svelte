<script lang="ts">
  // E15 menu : à propos, sécurité, légal, confidentialité, attributions, réglages.
  import { app, close, forgetPosition, eraseAll, open as openLayer, toast } from '../lib/app.svelte';
  import { i18n, t, setLang } from '../i18n/i18n.svelte';
  import Install from './Install.svelte';
  import { load, remove, save } from '../lib/store';

  // mesure d'audience (admin.md § 7) : opposition mémorisée sur l'appareil
  let stats = $state(!load('nostats', false));
  function pace(sec: number) { return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`; }
  const PACES = Array.from({ length: (720 - 240) / 15 + 1 }, (_, i) => 240 + i * 15);
</script>

<div class="head">
  <button class="icon-btn" onclick={close} aria-label={t().menu.close}>✕</button>
  <h2>{t().menu.open}</h2>
</div>

<button class="btn secondary wide" onclick={async () => { await close(); openLayer('history'); }}>{t().history.title}</button>
<Install />

<section>
  <h3>{t().menu.settings}</h3>
  <div class="row">
    <span id="lang-l">{t().app.langLabel}</span>
    <div class="seg" role="radiogroup" aria-labelledby="lang-l">
      <button role="radio" aria-checked={i18n.lang === 'fr'} onclick={() => setLang('fr')} lang="fr">FR</button>
      <button role="radio" aria-checked={i18n.lang === 'en'} onclick={() => setLang('en')} lang="en">EN</button>
    </div>
  </div>
  <div class="row">
    <label for="m-pace">{t().more.pace} ({t().more.paceUnit})</label>
    <select id="m-pace" bind:value={app.settings.paceS}>{#each PACES as p}<option value={p}>{pace(p)}</option>{/each}</select>
  </div>
  <div class="row">
    <label for="m-loops">{t().more.loops}</label>
    <select id="m-loops" bind:value={app.settings.nLoops}>{#each [1, 2, 3, 4] as n}<option value={n}>{n}</option>{/each}</select>
  </div>
  <div class="row">
    <label for="m-stats">{t().menu.stats}</label>
    <input id="m-stats" type="checkbox" checked={stats} onchange={(e) => { stats = e.currentTarget.checked; if (stats) remove('nostats'); else save('nostats', true); }} />
  </div>
  <button class="btn secondary wide" onclick={() => { forgetPosition(); toast(t().menu.forgotten); }}>{t().menu.forgetPosition}</button>
  <button class="btn danger wide" onclick={() => { if (confirm(t().menu.confirmErase)) eraseAll(); }}>{t().menu.eraseAll}</button>
</section>

<details open><summary>{t().menu.safety}</summary><p>{t().safety}</p></details>
<details><summary>{t().menu.about}</summary><p>{t().menu.aboutText}</p></details>
<details><summary>{t().menu.legal}</summary><p>{t().menu.legalText}</p></details>
<details><summary>{t().menu.privacy}</summary><p>{t().menu.privacyText}</p></details>
<details><summary>{t().menu.attributions}</summary><p>{t().menu.attributionsText}</p></details>
<a class="source" href="https://github.com/M-Garrigues/trail_opt" target="_blank" rel="noopener noreferrer">
  <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 7l-5 5 5 5M16 7l5 5-5 5" /></svg>{t().menu.source}
</a>

<style>
  .source { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--border); padding: 12px 0; font-weight: 600; color: var(--accent-text); min-height: 24px; }
  .head { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; }
  h2 { font-size: 1.25rem; margin: 0; }
  h3 { font-size: 1rem; margin: 16px 0 6px; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 8px; margin: 8px 0; }
  .row input[type='checkbox'] { width: 24px; height: 24px; margin: 10px; accent-color: var(--accent); }
  .seg { display: flex; border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }
  .seg button { all: unset; min-width: 52px; min-height: 44px; text-align: center; cursor: pointer; font-weight: 600; }
  .seg button[aria-checked='true'] { background: var(--accent); color: var(--on-accent); }
  .seg button:focus-visible { outline: 3px solid var(--accent); outline-offset: -3px; }
  section .btn { margin-top: 8px; }
  details { border-top: 1px solid var(--border); padding: 4px 0; }
  summary { cursor: pointer; padding: 10px 0; font-weight: 600; min-height: 24px; }
  details p { line-height: 1.5; margin: 0 0 8px; white-space: pre-line; } /* paragraphes séparés par \n\n (T30) */
</style>
