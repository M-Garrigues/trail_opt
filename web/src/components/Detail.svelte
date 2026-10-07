<script lang="ts">
  // E8 détail : profil coloré par pente + légende, chiffres, montées, avertissements, version des données.
  import { isImportant, app, close, current, open as openLayer, closeTo, mapUi, viaIdx, track } from '../lib/app.svelte';
  import { legs, idxAt } from '../lib/geo';
  import { LM_PATH } from '../lib/icons';
  import { durationMin, lowSurface } from '../lib/settings';
  import { saveGpx, canShareGpx } from '../lib/gpx';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, km, duration, warningText, climbsText, surfaceText, lowSurfaceText } from '../i18n/format';
  import Profile, { profileMarks } from './Profile.svelte';

  let { embedded = false }: { embedded?: boolean } = $props();
  const c = $derived(current()!);
  const res = $derived(app.result!);
  const L = $derived(i18n.lang);
  const kmEffort = $derived(c.length_m / 1000 + c.dplus_m / 100);
  // type de voie : l'avertissement est recalculé pour la sortie AFFICHÉE (celui du serveur vaut pour la première)
  const warnings = $derived([...res.warnings.filter((w) => !isImportant(w.code) && w.code !== 'low_surface_share').map((w) => warningText(L, w.code, w.params ?? {})),
    lowSurfaceText(lowSurface(c, app.request), L)].filter(Boolean));
  const surface = $derived(surfaceText(c, L));

  // D34 : tronçons départ → points de passage → arrivée (seulement s'il y a des points)
  const vi = $derived(viaIdx(c));
  const rows = $derived(vi.length ? legs(c.dist, c.ele, vi) : []);
  const vs = $derived([...vi].sort((a, b) => a - b));
  const name = (k: number, n: number) => (k === 0 ? t().detail.start : k === n ? t().detail.finish : `P${k}`);

  async function replan() {
    // « Recalculer depuis ici » : réglages et départ déjà remplis, retour à E3
    await closeTo('result');
  }
</script>

{#if c}
  {#if !embedded}
  <div class="head">
    <button class="icon-btn" onclick={close} aria-label={t().result.back}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
    </button>
    <h2>{t().detail.title}</h2>
  </div>
  {/if}
  {#if !app.dock}<Profile cand={c} bind:cursor={app.cursor} height={170} onpick={mapUi.center} marks={profileMarks(c, vi)} />{/if}
  <div class="actions-grid">
    <button class="btn primary" onclick={() => { track('gpx'); void saveGpx(c, res.data_version); }}>{canShareGpx(c) ? t().detail.gpxShare : t().detail.gpx}</button>
    {#if c.sig || app.shared}<button class="btn secondary" onclick={() => openLayer('share')} disabled={app.offline}>{t().result.share}</button>{/if}
    {#if !embedded}<button class="btn secondary" onclick={replan}>{t().detail.replan}</button>{/if}
  </div>
  <dl class="grid">
    <div><dt>{t().detail.distance}</dt><dd>{km(L, c.length_m, 2)}</dd></div>
    <div><dt>{t().detail.dplus}</dt><dd>+{num(L, c.dplus_m)} m</dd></div>
    <div><dt>{t().detail.altMin}</dt><dd>{num(L, c.alt_min_m)} m</dd></div>
    <div><dt>{t().detail.altMax}</dt><dd>{num(L, c.alt_max_m)} m</dd></div>
    <div><dt>{t().detail.maxGrade}</dt><dd>{num(L, c.max_grade_pct)} %</dd></div>
    <div><dt>{t().detail.kmEffort}</dt><dd>{num(L, kmEffort, 1)}</dd></div>
    <div><dt>{t().detail.duration}</dt><dd>{duration(durationMin(c.length_m / 1000, c.dplus_m, app.settings.paceS))}</dd></div>
  </dl>
  {#if surface}
    <h3>{t().surface.label}</h3>
    <p class="climbs" data-testid="surface-detail">{surface}</p>
  {/if}
  {#if c.climbs}
    <h3>{t().detail.climbs}</h3>
    <p class="climbs" data-testid="climbs-detail">{climbsText(c, L)}{#if c.climbs.count > 0}<br /><span class="muted">{t().detail.typical({ g: num(L, c.climbs.gbar_m) })}</span>{/if}</p>
    <p class="def">{t().detail.climbsDef}</p>
  {/if}
  {#if rows.length}
    <h3>{t().detail.legs}</h3>
    <table class="legs">
      <tbody>
        {#each rows as r, k}
          <tr><th scope="row"><button class="pt" onclick={() => mapUi.center(vs[Math.min(k, vs.length - 1)])} title={t().detail.centerOn}>{t().detail.leg({ a: name(k, rows.length), b: name(k + 1, rows.length) })}</button></th><td>{km(L, r.length_m)}</td><td>+{num(L, r.dplus_m)} m</td></tr>
        {/each}
      </tbody>
    </table>
  {/if}
  {#if c.landmarks?.length}
    <h3>{t().detail.landmarks}</h3>
    <ul class="marks">
      {#each c.landmarks as l}
        <li><button class="pt lm" onclick={() => mapUi.center(idxAt(c, l.dist_m))} title={t().detail.centerOn}>
          <span class="landmark {l.kind}" aria-hidden="true"><svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={LM_PATH[l.kind]} /></svg></span>
          <span class="nm">{l.name}<span class="sr"> ({t().detail[l.kind]})</span><small>{l.ele_m != null ? `≈ ${num(L, Math.round(l.ele_m / 10) * 10)} m · ` : ''}km {num(L, l.dist_m / 1000, 1)}</small></span>
        </button></li>
      {/each}
    </ul>
  {/if}
  {#if warnings.length}<ul class="warnings">{#each warnings as w}<li>{w}</li>{/each}</ul>{/if}
  <p class="version">{t().detail.data} : {res.data_version} · {res.solver_version}</p>
{/if}

<style>
  .head { display: flex; gap: 8px; align-items: center; }
  h2 { font-size: 1.25rem; margin: 0; }
  h3 { font-size: 1rem; margin: 12px 0 4px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 8px; margin: 10px 0; }
  .grid div { background: var(--surface); border-radius: 8px; padding: 8px; }
  dt { font-size: 0.85rem; color: var(--muted); }
  dd { margin: 0; font-weight: 700; font-size: 1.1rem; }
  .climbs { margin: 0; }
  .muted, .def { color: var(--muted); }
  .def { font-size: 0.85rem; margin: 4px 0 0; }
  .legs { border-collapse: collapse; width: 100%; }
  .legs th { text-align: left; font-weight: 600; }
  .legs th, .legs td { padding: 6px 4px; border-bottom: 1px solid var(--border); }
  .legs td { text-align: right; font-variant-numeric: tabular-nums; }
  .pt { all: unset; box-sizing: border-box; display: flex; align-items: center; gap: 8px; min-height: 32px; cursor: pointer; text-align: left; font-weight: inherit; }
  .pt:hover { color: var(--accent-text); text-decoration: underline; }
  .pt:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
  .marks { list-style: none; padding: 0; margin: 0; }
  .lm { width: 100%; gap: 10px; padding: 4px 0; border-bottom: 1px solid var(--border); min-height: 40px; }
  .lm .landmark { width: 24px; height: 24px; box-shadow: none; flex: none; }
  .lm .nm { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .lm .nm small { display: block; color: var(--muted); font-size: 0.8rem; font-variant-numeric: tabular-nums; }
  .marks li { display: flex; align-items: center; gap: 8px; min-height: 32px; }
  .warnings { color: var(--warn-text); padding-left: 1.2em; }
  .actions-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin: 12px 0; }
  .version { font-size: 0.8rem; color: var(--muted); }
</style>
