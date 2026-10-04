<script lang="ts">
  // E2 départ + E3 réglages + E5 calcul (ui-spec §3).
  import { app, compute, cancel, locate, setStart, has, open as openLayer } from '../lib/app.svelte';
  import { CATALOG, byId, clampField, type Field, type TypeId } from '../lib/catalog';
  import { expectedKm, durationMin, MAX_GRADES } from '../lib/settings';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, duration } from '../i18n/format';
  import Search from './Search.svelte';
  import TypePicker from './TypePicker.svelte';
  import { nature } from '../lib/design';

  let { onfly }: { onfly: (p: { lat: number; lon: number }) => void } = $props();

  const s = $derived(app.settings);
  const type = $derived(byId(s.typeId));
  const computing = $derived(has('computing'));
  let clampMsg = $state<Record<string, string>>({});
  let locating = $state(false);

  const est = $derived.by(() => {
    const v = s.values[type.id];
    return duration(durationMin(expectedKm(s), type.goal === 'max_dplus' ? 0 : (v.dplus_m ?? 0), s.paceS));
  });

  function setType(id: TypeId) {
    app.settings.typeId = id;
    clampMsg = {};
  }
  function cardsKey(e: KeyboardEvent) {
    const i = CATALOG.findIndex((c) => c.id === s.typeId);
    let j = i;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') j = (i + 1) % CATALOG.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') j = (i - 1 + CATALOG.length) % CATALOG.length;
    else return;
    e.preventDefault();
    setType(CATALOG[j].id);
    (document.getElementById(`type-${CATALOG[j].id}`) as HTMLElement | null)?.focus();
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
  function pace(sec: number) { return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`; }
  const PACES = Array.from({ length: (720 - 240) / 15 + 1 }, (_, i) => 240 + i * 15);

  // barre de progression : 90 % à l'estimation, puis pulsation
  let now = $state(performance.now());
  $effect(() => {
    if (!computing) return;
    let raf = requestAnimationFrame(function tick() { now = performance.now(); raf = requestAnimationFrame(tick); });
    return () => cancelAnimationFrame(raf);
  });
  const pct = $derived(Math.min(90, (90 * (now - app.progress.t0)) / 1000 / app.progress.estS));
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
  {:else if app.start}✓ {t().start.set} · {num(i18n.lang, app.start.lat, 4)}, {num(i18n.lang, app.start.lon, 4)}
  {:else}{t().start.tapMap}{/if}
</p>

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
            onchange={(e) => commit(f, (e.currentTarget as HTMLInputElement).value)}
            aria-describedby="m-{f.param}"
          />
        </div>
        <p id="m-{f.param}" class="field-msg" aria-live="polite">{clampMsg[f.param] ?? ''}</p>
      </div>
    {/each}
  {/snippet}
  {#if nature}
    <TypePicker value={s.typeId} onselect={setType} fields={typeFields} />
  {:else}
  <div class="cards" role="radiogroup" aria-label={t().types.label} tabindex="-1" onkeydown={cardsKey}>
      {#each CATALOG.filter((c) => c.enabled) as c (c.id)}
        <button
          id="type-{c.id}"
          class="card"
          role="radio"
          aria-checked={s.typeId === c.id}
          tabindex={s.typeId === c.id ? 0 : -1}
          onclick={() => setType(c.id)}
        >
          <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true"><path d={c.icon} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
          <strong>{t().types[c.id].name}</strong>
          <span>{t().types[c.id].tagline}</span>
        </button>
      {/each}
    </div>
    {@render typeFields()}
  {/if}

  <div class="field">
    <span class="label" id="climbs-l">{t().climbs.label}</span>
    <div class="seg" role="radiogroup" aria-labelledby="climbs-l">
      {#each ['short', 'balanced', 'long'] as const as c}
        <label class:on={s.climbs === c}><input type="radio" name="climbs" value={c} bind:group={app.settings.climbs} />{t().climbs[c]}</label>
      {/each}
    </div>
    {#if s.climbs === 'short'}<p class="note">{t().climbs.shortNote}</p>{/if}
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
    <div class="field">
      <label for="loops">{t().more.loops}</label>
      <select id="loops" bind:value={app.settings.nLoops}>
        {#each [1, 2, 3, 4] as n}<option value={n}>{n}</option>{/each}
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
  <div class="actions">
    <button class="btn secondary" onclick={() => openLayer('zone')} aria-pressed={!!app.zone} disabled={!app.start}>
      {app.zone ? t().zone.set : t().zone.button}
    </button>
    <button class="btn primary grow" onclick={() => compute('new')} disabled={!app.start || app.offline} aria-describedby={app.start ? undefined : 'need-start'}>
      {t().compute.find}
    </button>
  </div>
  {#if !app.start}<p id="need-start" class="sr">{t().compute.needStart}</p>{/if}
{/if}
</div>

<style>
  .start-row { display: flex; gap: 8px; align-items: center; }
  .hint { margin: 6px 0 10px; color: var(--muted); min-height: 1.4em; }
  .settings { border: 0; padding: 0; margin: 0; min-width: 0; }
  .cards { display: flex; gap: 10px; overflow-x: auto; scroll-snap-type: x mandatory; padding: 2px 2px 8px; margin: 0 -2px; }
  .card {
    flex: 0 0 auto; width: 150px; scroll-snap-align: start; text-align: left; display: flex; flex-direction: column;
    gap: 4px; padding: 12px; border: 2px solid var(--border); border-radius: 12px; background: var(--surface); color: var(--text);
    font: inherit; cursor: pointer;
  }
  .card span { font-size: 0.875rem; color: var(--muted); line-height: 1.25; }
  .card[aria-checked='true'] { border-color: var(--accent); background: var(--accent-soft); }
  .card[aria-checked='true'] svg { color: var(--accent); }
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
  .note { margin: 6px 0 0; font-size: 0.9rem; color: var(--muted); }
  .more summary { cursor: pointer; padding: 10px 0; min-height: 24px; font-weight: 600; color: var(--accent-text); }
  .check label { display: flex; gap: 8px; align-items: center; min-height: 44px; font-weight: 400; }
  .estimate { margin: 8px 0; }
  .bar { position: sticky; bottom: 0; background: var(--sheet-bg, var(--bg)); transition: background-color 0.4s; padding: 8px 0 12px; border-top: 1px solid var(--border); margin: 0 -16px; padding-inline: 16px; }
  .actions { display: flex; gap: 8px; }
  .grow { flex: 1; }
  .progress-wrap p { margin: 4px 0 8px; font-weight: 600; }
  .progress { height: 10px; border-radius: 5px; background: var(--border); overflow: hidden; margin-bottom: 10px; }
  .progress span { display: block; height: 100%; background: var(--accent); }
  .progress.pulse span { animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.45; } }
  @media (prefers-reduced-motion: reduce) { .progress.pulse span { animation: none; } }
  .err-inline { color: var(--danger); }
</style>
