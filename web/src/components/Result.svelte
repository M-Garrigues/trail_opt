<script lang="ts">
  // E6 résultat + E7 autres boucles.
  import { isImportant, app, compute, select, open as openLayer, close, current, mapUi, debug, viaIdx } from '../lib/app.svelte';
  import { saveGpx, canShareGpx } from '../lib/gpx';
  import { byId } from '../lib/catalog';
  import { durationMin, lowSurface } from '../lib/settings';
  import { LOOP_COLORS } from '../lib/map';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, km, duration, warningText, climbsText, surfaceText, lowSurfaceText } from '../i18n/format';
  import Profile, { profileMarks } from './Profile.svelte';
  import Detail from './Detail.svelte';


  const c = $derived(current()!);
  const res = $derived(app.result!);
  const type = $derived(byId(app.settings.typeId));
  const L = $derived(i18n.lang);
  // type de voie : l'avertissement est recalculé pour la sortie AFFICHÉE (celui du serveur vaut pour la première)
  const warnings = $derived([...res.warnings.filter((w) => !isImportant(w.code) && w.code !== 'low_surface_share').map((w) => warningText(L, w.code, w.params ?? {})),
    lowSurfaceText(lowSurface(c, app.request), L)].filter(Boolean));
  const surface = $derived(surfaceText(c, L));
  const extra = $derived(app.cands.length - 1);
  const climbs = $derived(climbsText(c, L));
</script>

{#snippet loops()}
  {#if extra > 0}
    <section class="others" class:wide={app.wide} aria-label={t().result.otherLoops}>
      <h3 class="other-title">{t().result.otherLoopsN({ n: String(extra) })}</h3>
      <div class="loops" role="radiogroup" aria-label={t().result.otherLoops}>
        {#each app.cands as k, i}
          <button class="loop" role="radio" aria-checked={i === app.sel} onclick={() => select(i)} aria-label={t().result.choose({ n: String(i + 1) })}
            style:--i={i} style:--c={LOOP_COLORS[i]}>
            <span class="badge" style:background={LOOP_COLORS[i]}>{i + 1}</span>
            <span>{km(L, k.length_m)}<br />+{num(L, k.dplus_m)} m</span>
          </button>
        {/each}
      </div>
    </section>
  {/if}
{/snippet}

{#if c}
  <div class="head">
    <button class="icon-btn" onclick={close} aria-label={t().result.back} title={t().result.back}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
    </button>
    <span class="badge" style:background={LOOP_COLORS[app.sel]}>{app.sel + 1}</span>
    <div>
      <p class="headline" data-testid="headline">{type.headline(c, res, L)}</p>
      {#if type.goal === 'min_distance' && res.lower_bound_m}
        <p class="sub">{t().result.noneUnder({ km: num(L, res.lower_bound_m / 1000, 1) })}</p>
      {/if}
    </div>
  </div>
  <p class="stats" data-testid="stats">
    {km(L, c.length_m)} · +{num(L, c.dplus_m)} m · {num(L, c.dplus_m / (c.length_m / 1000))} {t().result.perKm} · {duration(durationMin(c.length_m / 1000, c.dplus_m, app.settings.paceS))}
  </p>
  {#if warnings.length && !app.wide}
    <ul class="warnings">{#each warnings as w}<li>{w}</li>{/each}</ul>
  {/if}

  {#if app.wide}
    <!-- bureau : détail déplié (profil dans la bande du bas), propositions en bas du panneau sans défilement -->
    <Detail embedded />
    {#if !app.fromHistory}
      <button class="btn secondary wide-btn" onclick={() => compute('seed')}>{t().result.newSuggestions}</button>
    {/if}
  {:else}
    {#if climbs}<p class="climbs" data-testid="climbs">{climbs}</p>{/if}
    {#if surface}<p class="climbs" data-testid="surface">{surface}</p>{/if}
    <Profile cand={c} bind:cursor={app.cursor} height={110} onpick={mapUi.center} marks={profileMarks(c, viaIdx(c))} />
    <p class="hint">{t().detail.centerHint}</p>
  {/if}

  {@render loops()}
  {#if app.noMore}<p class="muted">{t().result.noOther}</p>{/if}
  {#if debug}
    <p class="debug" data-testid="debug">
      data {res.data_version} · solver {res.solver_version} · {num('en', res.compute_s, 2)} s server{app.fromHistory ? '' : ` · ${num('en', app.clientS, 2)} s client`}
      · start {res.effective_start.kind}{res.warnings.length ? ` · ${res.warnings.map((w) => w.code).join(', ')}` : ''}
    </p>
  {/if}
  {#if !app.fromHistory && app.cands.length < 2 && !app.noMore}
    <button class="btn secondary wide" onclick={() => compute('more')}>{t().result.otherLoops}</button>
  {/if}
  {#if !app.wide}
    <!-- mobile : actions toujours visibles au bas de la feuille (une rangée, le contenu défile au-dessus) -->
    <div class="bar">
      <button class="btn secondary ico" onclick={() => openLayer('detail')} aria-label={t().result.details}>
        <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M9 6h11M9 12h11M9 18h11" /><circle cx="4.500" cy="6" r="1" /><circle cx="4.500" cy="12" r="1" /><circle cx="4.500" cy="18" r="1" /></svg>
        <span class="cap">{t().result.details}</span>
      </button>
      {#if c.sig || app.shared}
        <button class="btn secondary ico" onclick={() => openLayer('share')} disabled={app.offline} aria-label={t().result.share}>
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 15V3M8 7l4-4 4 4M8 11H6v10h12V11h-2" /></svg>
          <span class="cap">{t().result.share}</span>
        </button>
      {/if}
      {#if !app.fromHistory}
        <button class="btn secondary ico" onclick={() => compute('seed')} aria-label={t().result.newSuggestions}>
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 12a8 8 0 1 1-2.300-5.600M20 4v5h-5" /></svg>
          <span class="cap">{t().result.again}</span>
        </button>
      {/if}
      <button class="btn primary gpx" onclick={() => saveGpx(c, res.data_version)}>{canShareGpx(c) ? t().detail.gpxShare : t().detail.gpx}</button>
    </div>
  {/if}
{/if}

<style>
  .head { display: flex; gap: 10px; align-items: center; }
  .headline { font-size: clamp(1.2rem, 5.6vw, 1.6rem); font-weight: 800; margin: 0; line-height: 1.1; }
  .sub { margin: 2px 0 0; color: var(--muted); }
  .stats { margin: 6px 0 2px; font-weight: 600; }
  .climbs { margin: 0 0 6px; color: var(--muted); }
  .warnings { margin: 6px 0; padding-left: 1.2em; color: var(--warn-text); }
  .bar { position: sticky; bottom: 0; z-index: 11; display: flex; gap: 8px; margin: auto -16px 0; padding: 8px 16px 12px; background: var(--here, var(--lvl-2));
    border-top: 1px solid var(--border); box-shadow: 0 -6px 12px -8px rgb(var(--sh-rgb) / 0.25); }
  .gpx { flex: 1; min-width: 0; padding-inline: 6px; white-space: nowrap; }
  .other-title { font-size: 1rem; margin: 12px 0 6px; }
  .loops { display: flex; gap: 8px; overflow-x: auto; padding-bottom: 4px; }
  .loop {
    display: flex; gap: 8px; align-items: center; padding: 8px 12px; border: 2px solid var(--border); border-radius: 10px;
    background: var(--surface); color: var(--text); font: inherit; cursor: pointer; min-height: 44px; text-align: left; flex: none;
  }
  .loop[aria-checked='true'] { border-color: var(--accent); }
  .muted { color: var(--muted); }
  .debug { font: 0.8rem ui-monospace, monospace; color: var(--muted); overflow-wrap: anywhere; }
  .hint { margin: 2px 0 0; font-size: 0.85rem; color: var(--muted); }
  .wide-btn { margin: 4px 0 10px; }
  /* bureau : propositions collées en bas du panneau (visibles sans défiler) */
  .others.wide { position: sticky; bottom: 0; margin: auto -16px 0; padding: 4px 16px 12px; background: var(--here, var(--lvl-2)); border-top: 1px solid var(--border); z-index: 1; }
  .others.wide .other-title { margin-top: 8px; }
  /* b : « strates » empilées, la boucle choisie est la plus haute (ombre plus large, décalée) */
  .others .loops { display: flex; flex-direction: column; gap: 0; overflow: visible; padding: 0 0 4px; }
  .others .loop { width: 100%; border: 1px solid var(--border); border-left: 6px solid var(--c); background: var(--lvl-1); position: relative; margin-top: -6px; border-radius: 12px;
    z-index: calc(1 + var(--i)); box-shadow: var(--sh-1); transition: transform 0.35s var(--ease), box-shadow 0.35s var(--ease), background-color 0.35s; }
  .others .loop:first-child { margin-top: 0; }
  .others .loop[aria-checked='true'] { transform: translateY(-6px) translateX(4px); background: var(--lvl-3); box-shadow: var(--sh-3); z-index: 10; border-color: var(--border); }
  .others .loop br { display: none; }
  .others .loop span:last-child { display: flex; gap: 12px; font-variant-numeric: tabular-nums; }
  @media (prefers-reduced-motion: reduce) { .others .loop { transition: none; } }
</style>
