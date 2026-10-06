<script lang="ts">
  // E2 départ + E3 réglages + E5 calcul (ui-spec §3).
  import { app, compute, cancel, locate, setStart, has, open as openLayer } from '../lib/app.svelte';
  import { byId, clampField, type Field, type TypeId } from '../lib/catalog';
  import { expectedKm, durationMin, MAX_GRADES } from '../lib/settings';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, duration } from '../i18n/format';
  import Search from './Search.svelte';
  import TypePicker from './TypePicker.svelte';
  import { VIA, VIA_MAX, removeVia } from '../lib/app.svelte';

  let { onfly, desktop = false }: { onfly: (p: { lat: number; lon: number }) => void; desktop?: boolean } = $props();

  const s = $derived(app.settings);
  const type = $derived(byId(s.typeId));
  const computing = $derived(has('computing'));
  let clampMsg = $state<Record<string, string>>({});
  let locating = $state(false);

  const est = $derived.by(() => {
    const v = s.values[type.id];
    return duration(durationMin(expectedKm(s), type.goal === 'max_dplus' ? 0 : (v.dplus_m ?? 0), s.paceS));
  });

  // Barre du bas (mobile, D34) : résumé toujours visible des réglages, ouvre la feuille dessus.
  const summary = $derived.by(() => {
    const v = s.values[type.id], L = i18n.lang;
    const parts = [t().types[type.id].name];
    for (const f of type.fields) if (v[f.param] != null) parts.push(`${f.param === 'dplus_m' ? '+' : ''}${num(L, v[f.param]!, f.unit === 'km' ? 1 : 0)} ${f.unit}`);
    if (s.climbs !== 'balanced') parts.push(t().summary.climbs[s.climbs]);
    if (app.zone) parts.push(t().zone.button);
    return parts.join(' · ');
  });
  function showSettings() {
    if (app.snap === 2) { app.snap = 0; return; } // 2 états : un nouvel appui referme tout
    app.snap = 2;
    // le type choisi (et ses réglages) en vue
    setTimeout(() => document.getElementById(`type-${s.typeId}`)?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' }), 50);
  }
  function setType(id: TypeId) {
    app.settings.typeId = id;
    clampMsg = {};
  }
  function commit(f: Field, raw: string) {
    const v = parseFloat(raw.replace(',', '.'));
    if (f.optional && raw.trim() === '') { app.settings.values[type.id][f.param] = null; return; }
    const r = clampField(f, v);
    app.settings.values[type.id][f.param] = r.v;
    const fmt = (x: number) => `${num(i18n.lang, x, 1)} ${f.unit}`;
    clampMsg[f.param] = r.changed && Number.isFinite(v) && (v < f.min || v > f.max)
      ? t().fields.clamped({ v: fmt(r.v), min: fmt(f.min), max: fmt(f.max) }) : '';
  }
  async function myLocation() {
    locating = true;
    const p = await locate();
    locating = false;
    if (p) onfly(p);
  }
  // mobile : le clavier virtuel réduit visualViewport ; la feuille est ajustée (App, --kb) et le champ remonté au-dessus
  function keepVisible(el: HTMLElement) {
    for (const d of [60, 350]) setTimeout(() => el.scrollIntoView({ block: 'center', behavior: 'auto' }), d);
  }
  function pace(sec: number) { return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`; }
  const PACES = Array.from({ length: (720 - 240) / 15 + 1 }, (_, i) => 240 + i * 15);

  // barre de progression : ~90 % à l'estimation, puis asymptotique (jamais 100 % avant la réponse)
  let now = $state(performance.now());
  $effect(() => {
    if (!computing) return;
    let raf = requestAnimationFrame(function tick() { now = performance.now(); raf = requestAnimationFrame(tick); });
    return () => cancelAnimationFrame(raf);
  });
  const pct = $derived(99 * (1 - Math.exp((-2.3 * (now - app.progress.t0)) / 1000 / app.progress.estS)));
</script>

<div class="start-row">
  <Search onpick={(h) => { setStart(h); onfly(h); }} />
  <button class="icon-btn" onclick={myLocation} disabled={locating || computing} aria-label={t().start.myLocation} title={t().start.myLocation}>
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><circle cx="12" cy="12" r="4" fill="currentColor" /><circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" stroke-width="2" /><path d="M12 1v4M12 19v4M1 12h4M19 12h4" stroke="currentColor" stroke-width="2" /></svg>
  </button>
</div>
<p class="hint" aria-live="polite">
  {#if locating}{t().start.locating}
  {:else if app.geoError}<span class="err-inline">{t().geo.denied}</span>
  {:else if app.placing}{t().via.hint({ max: String(VIA_MAX) })}
  {:else if app.start}✓ {t().start.set} · {num(i18n.lang, app.start.lat, 4)}, {num(i18n.lang, app.start.lon, 4)}
  {:else}{t().start.tapMap}{/if}
</p>
{#if app.via.length}
  <ol class="via-list" aria-label={t().via.list}>
    {#each app.via as _, i}
      <li><span class="via-dot" aria-hidden="true">{i + 1}</span>{t().via.point({ n: String(i + 1) })}
        <button class="icon-btn small" onclick={() => removeVia(i)} aria-label={t().via.remove({ n: String(i + 1) })} disabled={computing}>✕</button></li>
    {/each}
  </ol>
{/if}

<fieldset class="settings" disabled={computing}>
  <legend class="sr">{t().types.label}</legend>
  {#snippet typeFields()}
  {#each type.fields as f (type.id + f.param)}
      {@const v = s.values[type.id][f.param]}
      <div class="field">
        <label for="f-{f.param}">{t().fields[f.param]} <span class="unit">({f.unit})</span></label>
        <div class="num-row">
          <input
            id="f-{f.param}"
            type="number"
            inputmode="decimal"
            min={f.min}
            max={f.max}
            step={f.step}
            value={v ?? ''}
            placeholder={f.auto ? t().fields.auto({ km: num(i18n.lang, f.auto(s.values[type.id]), 1) }) : ''}
            onfocus={(e) => keepVisible(e.currentTarget as HTMLElement)}
            onchange={(e) => commit(f, (e.currentTarget as HTMLInputElement).value)}
            aria-describedby="m-{f.param}"
          />
        </div>
        <p id="m-{f.param}" class="field-msg" aria-live="polite">{clampMsg[f.param] ?? ''}</p>
      </div>
    {/each}
  {/snippet}
  <TypePicker value={s.typeId} onselect={setType} fields={typeFields} />

  <div class="field">
    <span class="label" id="climbs-l">{t().climbs.label}</span>
    <div class="seg" role="radiogroup" aria-labelledby="climbs-l" aria-describedby="climbs-help">
      {#each ['short', 'balanced', 'long'] as const as c}
        <label class:on={s.climbs === c}><input type="radio" name="climbs" value={c} bind:group={app.settings.climbs} />{t().climbs[c]}</label>
      {/each}
    </div>
    <p class="note" id="climbs-help">{s.typeId === 'target' ? t().climbs.helpTarget : t().climbs.help[s.climbs]}</p>
  </div>

  <div class="field">
    <label for="loops">{t().more.loops} : <strong>{s.nLoops}</strong></label>
    <input id="loops" type="range" min="1" max="4" step="1" bind:value={app.settings.nLoops} aria-valuetext={String(s.nLoops)} />
  </div>

  <details class="more">
    <summary>{t().more.title}</summary>
    <div class="field">
      <label for="max-grade">{t().more.maxGrade}</label>
      <select id="max-grade" bind:value={app.settings.maxGrade}>
        {#each MAX_GRADES as g}<option value={g}>{g ? `${g} %` : t().more.noLimit}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="roads">{t().more.roads}</label>
      <select id="roads" bind:value={app.settings.roads}>
        <option value="unpaved">{t().more.unpaved}</option>
        <option value="minor">{t().more.minor}</option>
        <option value="all">{t().more.all}</option>
      </select>
    </div>
    <div class="field check">
      <label><input type="checkbox" bind:checked={app.settings.noRepeat} /> {t().more.noRepeat}</label>
    </div>
    <div class="field">
      <label for="pace">{t().more.pace} ({t().more.paceUnit})</label>
      <select id="pace" bind:value={app.settings.paceS}>
        {#each PACES as p}<option value={p}>{pace(p)}</option>{/each}
      </select>
    </div>
  </details>
</fieldset>

<p class="estimate">{t().estimate} : <strong>{est}</strong></p>

<div class="bar">
{#if computing}
  <div class="progress-wrap" aria-live="polite">
    <p>{t().compute.computing}</p>
    <div class="progress" class:pulse={pct >= 90} role="progressbar" aria-label={t().compute.computing} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(pct)}>
      <span style:width="{pct}%"></span>
    </div>
    <button class="btn secondary wide" onclick={cancel}>{t().compute.cancel}</button>
  </div>
{:else}
  {#if !desktop}
    <button class="summary" onclick={showSettings} aria-label="{t().summary.open} : {summary}">
      <span class="sum-txt">{summary}</span><span class="sum-cta">{t().summary.settings}<svg class="chev" class:down={app.snap === 2} viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M6 15l6-6 6 6" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" /></svg></span>
    </button>
  {/if}
  <div class="actions" class:icons={!desktop}>
    <button class="btn secondary" class:ico={!desktop} onclick={() => openLayer('zone')} aria-pressed={!!app.zone} disabled={!app.start}
      aria-label={app.zone ? t().zone.set : t().zone.button} title={app.zone ? t().zone.set : t().zone.button}>
      {#if desktop}{app.zone ? t().zone.set : t().zone.button}{:else}
        <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 6l10-2 5 8-4 8-11-2z" stroke-dasharray="3 3" /></svg>{#if app.zone}<i class="dot"></i>{/if}
      {/if}
    </button>
    {#if VIA}
      <button class="btn secondary" class:ico={!desktop} onclick={() => (app.placing = !app.placing)} aria-pressed={app.placing} disabled={!app.start}
        aria-label="{t().via.button}{app.via.length ? ` (${app.via.length})` : ''}" title={t().via.button}>
        {#if desktop}{t().via.button}{app.via.length ? ` (${app.via.length})` : ''}{:else}
          <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="4" cy="12" r="3" fill="currentColor" /><circle cx="20" cy="12" r="3" fill="currentColor" /><path d="M7.600 12h1.600M14.800 12h1.600" /><circle cx="12" cy="12" r="1.700" /></svg>{#if app.placing || app.via.length}<i class="dot"></i>{/if}
        {/if}
      </button>
    {/if}
    <button class="btn primary grow" class:ico={!desktop} onclick={() => compute('new')} disabled={!app.start || app.offline} aria-describedby={app.start ? undefined : 'need-start'}
      aria-label={t().compute.find} title={t().compute.find}>
      {#if desktop}{t().compute.find}{:else}
        <svg viewBox="0 0 24 24" width="38" height="38" aria-hidden="true" fill="none" stroke="currentColor" stroke-linejoin="round" stroke-linecap="round"><path d="M5.42 3.07Q3.50 2.00 3.50 4.20L3.50 19.80Q3.50 22.00 5.42 20.93L19.58 13.07Q21.50 12.00 19.58 10.93z" stroke-width="1.7" /><path d="M6.96 6.48Q5.73 5.80 5.73 7.20L5.73 16.80Q5.73 18.20 6.96 17.52L15.67 12.68Q16.89 12.00 15.67 11.32z" stroke-width="1.2" /><path d="M8.55 9.79Q7.85 9.40 7.85 10.20L7.85 13.80Q7.85 14.60 8.55 14.21L11.83 12.39Q12.53 12.00 11.83 11.61z" stroke-width="1" /></svg>
      {/if}
    </button>
  </div>
  {#if !app.start}<p id="need-start" class="sr">{t().compute.needStart}</p>{/if}
{/if}
</div>

<style>
  .start-row { display: flex; gap: 8px; align-items: center; }
  .hint { margin: 6px 0 10px; color: var(--muted); min-height: 1.4em; }
  .settings { border: 0; padding: 0; margin: 0; min-width: 0; }
  .field { margin: 10px 0; }
  .field > label, .label { display: block; font-weight: 600; margin-bottom: 4px; }
  .unit { font-weight: 400; color: var(--muted); }
  .num-row input { width: 100%; }
  .field-msg { margin: 2px 0 0; color: var(--danger); font-size: 0.9rem; min-height: 0; }
  .field-msg:empty { display: none; }
  .seg { display: flex; border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }
  .seg label { flex: 1; text-align: center; padding: 10px 4px; min-height: 44px; box-sizing: border-box; cursor: pointer; display: grid; place-items: center; font-size: 0.95rem; }
  .seg label + label { border-left: 1px solid var(--border); }
  .seg label.on { background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .seg input { position: absolute; opacity: 0; width: 1px; height: 1px; }
  .seg label:has(input:focus-visible) { outline: 3px solid var(--accent); outline-offset: -3px; }
  input[type='range'] { width: 100%; min-height: 32px; accent-color: var(--accent); }
  .note { margin: 6px 0 0; font-size: 0.9rem; color: var(--muted); }
  .more summary { cursor: pointer; padding: 10px 0; min-height: 24px; font-weight: 600; color: var(--accent-text); }
  .check label { display: flex; gap: 8px; align-items: center; min-height: 44px; font-weight: 400; }
  .estimate { margin: 8px 0; }
  .chev { transition: transform 0.25s; } .chev.down { transform: rotate(180deg); }
  .bar { position: sticky; bottom: 0; z-index: 1; margin-top: auto; background: var(--here, var(--lvl-2)); padding: 8px 16px 12px; border-top: 1px solid var(--border); margin: 0 -16px;
    box-shadow: 0 -6px 12px -8px rgb(var(--sh-rgb) / 0.25); }
  .summary { all: unset; box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: 8px; min-height: 44px; margin: -4px 0 6px; cursor: pointer; }
  .summary:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; border-radius: 8px; }
  .sum-txt { flex: 1; min-width: 0; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sum-cta { flex: none; display: flex; align-items: center; gap: 2px; color: var(--accent-text); font-weight: 700; text-decoration: underline; text-underline-offset: 3px; }
  .via-list { list-style: none; margin: -4px 0 8px; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
  .via-list li { display: flex; align-items: center; gap: 6px; padding-left: 6px; border: 1px solid var(--border); border-radius: 22px; background: var(--surface); }
  .via-dot { display: inline-grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--text); color: var(--bg); font-weight: 800; font-size: 0.85rem; }
  .icon-btn.small { border: 0; background: transparent; border-radius: 50%; }
  .actions { display: flex; gap: 8px; }
  .actions .btn.secondary { padding-inline: 12px; }
  .grow { flex: 1; }
  .actions.icons .grow { flex: 0 0 auto; min-width: 88px; margin-left: auto; }
  .btn.ico { position: relative; min-width: 48px; min-height: 48px; padding: 0 12px; display: inline-grid; place-items: center; }
  .btn.ico.secondary[aria-pressed='true'] { color: var(--accent-text); border-color: var(--accent); background: var(--accent-soft); }
  .dot { position: absolute; top: 6px; right: 6px; width: 9px; height: 9px; border-radius: 50%; background: var(--accent); border: 2px solid var(--bg); }
  .progress-wrap p { margin: 4px 0 8px; font-weight: 600; }
  .progress { height: 10px; border-radius: 5px; background: var(--border); overflow: hidden; margin-bottom: 10px; }
  .progress span { display: block; height: 100%; background: var(--accent); }
  .progress.pulse span { animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.45; } }
  @media (prefers-reduced-motion: reduce) { .progress.pulse span { animation: none; } }
  .err-inline { color: var(--danger); }
</style>
