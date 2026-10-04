<script lang="ts">
  // E8 détail : profil coloré par pente + légende, chiffres, montées, avertissements, version des données.
  import { app, close, current, open as openLayer, closeTo, mapUi } from '../lib/app.svelte';
  import { durationMin } from '../lib/settings';
  import { saveGpx, canShareGpx } from '../lib/gpx';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, km, duration, warningText } from '../i18n/format';
  import Profile from './Profile.svelte';

  const c = $derived(current()!);
  const res = $derived(app.result!);
  const L = $derived(i18n.lang);
  const kmEffort = $derived(c.length_m / 1000 + c.dplus_m / 100);
  const warnings = $derived(res.warnings.map((w) => warningText(L, w.code, w.params ?? {})).filter(Boolean));
  const LEGEND = [['0', '#4CAF50'], ['5', '#CDDC39'], ['10', '#FFC107'], ['18', '#F4511E'], ['28', '#C62828'], ['40+', '#5D0F0F']];

  async function replan() {
    // « Recalculer depuis ici » : réglages et départ déjà remplis, retour à E3
    await closeTo('result');
  }
</script>

{#if c}
  <div class="head">
    <button class="icon-btn" onclick={close} aria-label={t().result.back}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
    </button>
    <h2>{t().detail.title}</h2>
  </div>
  {#if !app.dock}<Profile cand={c} bind:cursor={app.cursor} height={170} onpick={mapUi.center} />{/if}
  <p class="legend" aria-label={t().detail.legend}>
    {t().detail.legend} :
    {#each LEGEND as [g, col]}<span><i style:background={col}></i>{g} %</span>{/each}
  </p>
  <dl class="grid">
    <div><dt>{t().detail.distance}</dt><dd>{km(L, c.length_m, 2)}</dd></div>
    <div><dt>{t().detail.dplus}</dt><dd>+{num(L, c.dplus_m)} m</dd></div>
    <div><dt>{t().detail.altMin}</dt><dd>{num(L, c.alt_min_m)} m</dd></div>
    <div><dt>{t().detail.altMax}</dt><dd>{num(L, c.alt_max_m)} m</dd></div>
    <div><dt>{t().detail.maxGrade}</dt><dd>{num(L, c.max_grade_pct)} %</dd></div>
    <div><dt>{t().detail.kmEffort}</dt><dd>{num(L, kmEffort, 1)}</dd></div>
    <div><dt>{t().detail.duration}</dt><dd>{duration(durationMin(c.length_m / 1000, c.dplus_m, app.settings.paceS))}</dd></div>
  </dl>
  {#if c.climbs}
    <h3>{t().detail.climbs}</h3>
    <ul class="climbs">
      <li>{t().detail.climbsCount({ n: num(L, c.climbs.count), plural: c.climbs.count > 1 ? 's' : '' })}</li>
      {#if c.climbs.count > 0}
        <li>{t().detail.longest({ gain: num(L, c.climbs.longest_gain_m), len: km(L, c.climbs.longest_len_m) })}</li>
        <li>{t().detail.typical({ g: num(L, c.climbs.gbar_m) })}</li>
      {/if}
    </ul>
  {/if}
  {#if warnings.length}<ul class="warnings">{#each warnings as w}<li>{w}</li>{/each}</ul>{/if}
  <div class="actions-grid">
    <button class="btn primary" onclick={() => saveGpx(c, res.data_version)}>{canShareGpx(c) ? t().detail.gpxShare : t().detail.gpx}</button>
    {#if c.sig || app.shared}<button class="btn secondary" onclick={() => openLayer('share')} disabled={app.offline}>{t().result.share}</button>{/if}
    <button class="btn secondary" onclick={replan}>{t().detail.replan}</button>
  </div>
  <p class="version">{t().detail.data} : {res.data_version} · {res.solver_version}</p>
{/if}

<style>
  .head { display: flex; gap: 8px; align-items: center; }
  h2 { font-size: 1.25rem; margin: 0; }
  h3 { font-size: 1rem; margin: 12px 0 4px; }
  .legend { display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: 0.85rem; color: var(--muted); margin: 6px 0; }
  .legend i { display: inline-block; width: 12px; height: 12px; border-radius: 2px; margin-right: 3px; vertical-align: -1px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 8px; margin: 10px 0; }
  .grid div { background: var(--surface); border-radius: 8px; padding: 8px; }
  dt { font-size: 0.85rem; color: var(--muted); }
  dd { margin: 0; font-weight: 700; font-size: 1.1rem; }
  .climbs { margin: 0; padding-left: 1.2em; }
  .warnings { color: var(--warn-text); padding-left: 1.2em; }
  .actions-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin: 12px 0; }
  .version { font-size: 0.8rem; color: var(--muted); }
</style>
