<script lang="ts">
  // E6 résultat + E7 autres boucles.
  import { app, compute, select, open as openLayer, close, current, mapUi, debug } from '../lib/app.svelte';
  import { saveGpx, canShareGpx } from '../lib/gpx';
  import { nature } from '../lib/design';
  import { byId } from '../lib/catalog';
  import { durationMin } from '../lib/settings';
  import { LOOP_COLORS } from '../lib/map';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, km, duration, warningText } from '../i18n/format';
  import Profile from './Profile.svelte';

  const c = $derived(current()!);
  const res = $derived(app.result!);
  const type = $derived(byId(app.settings.typeId));
  const L = $derived(i18n.lang);
  const warnings = $derived(res.warnings.map((w) => warningText(L, w.code, w.params ?? {})).filter(Boolean));
  const extra = $derived(app.cands.length - 1);
</script>

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
  {#if warnings.length}
    <ul class="warnings">{#each warnings as w}<li>{w}</li>{/each}</ul>
  {/if}

  {#if !app.dock}
    <Profile cand={c} bind:cursor={app.cursor} height={110} onpick={mapUi.center} />
    {#if nature}<p class="hint">{t().detail.centerHint}</p>{/if}
  {/if}

  <div class="actions-grid">
    <button class="btn secondary" onclick={() => openLayer('detail')}>{t().result.details}</button>
    <button class="btn primary" onclick={() => saveGpx(c, res.data_version)}>{canShareGpx(c) ? t().detail.gpxShare : t().detail.gpx}</button>
    {#if c.sig || app.shared}<button class="btn secondary" onclick={() => openLayer('share')} disabled={app.offline}>{t().result.share}</button>{/if}
    {#if !app.fromHistory}
      <button class="btn secondary" onclick={() => compute('seed')}>{t().result.newSuggestions}</button>
    {/if}
  </div>

  {#if extra > 0}
    <h3 class="other-title">{t().result.otherLoopsN({ n: String(extra) })}</h3>
    <div class="loops" role="radiogroup" aria-label={t().result.otherLoops}>
      {#each app.cands as k, i}
        <button class="loop" role="radio" aria-checked={i === app.sel} onclick={() => select(i)} aria-label={t().result.choose({ n: String(i + 1) })}>
          <span class="badge" style:background={LOOP_COLORS[i]}>{i + 1}</span>
          <span>{km(L, k.length_m)}<br />+{num(L, k.dplus_m)} m</span>
        </button>
      {/each}
    </div>
  {/if}
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
{/if}

<style>
  .head { display: flex; gap: 10px; align-items: center; }
  .headline { font-size: 1.6rem; font-weight: 800; margin: 0; line-height: 1.1; }
  .sub { margin: 2px 0 0; color: var(--muted); }
  .stats { margin: 6px 0; font-weight: 600; }
  .warnings { margin: 6px 0; padding-left: 1.2em; color: var(--warn-text); }
  .actions-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin: 10px 0; }
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
</style>
