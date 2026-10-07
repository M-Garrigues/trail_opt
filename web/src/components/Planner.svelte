<script lang="ts">
  // E2 départ + E3 réglages + E5 calcul (ui-spec §3).
  // Mobile : tout le parcours courant tient dans une feuille à hauteur de contenu, SANS clavier du système : type en
  // rangée de puces, chaque valeur est un bouton qui ouvre le pavé numérique intégré (à la place des réglages, même
  // hauteur de feuille), options dans une couche à part.
  import { app, compute, cancel, locate, setStart, has, top, close, open as openLayer, startOutsideZone } from '../lib/app.svelte';
  import { byId, clampField, type Field, type TypeId } from '../lib/catalog';
  import { expectedKm, durationMin, MAX_GRADES, defaultSettings } from '../lib/settings';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num, duration } from '../i18n/format';
  import Search from './Search.svelte';
  import TypePicker from './TypePicker.svelte';
  import Keypad from './Keypad.svelte';
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

  // Options hors défaut : pastille sur le bouton « Options » (mobile)
  const D = defaultSettings();
  const optsChanged = $derived(s.climbs !== D.climbs || s.nLoops !== D.nLoops || s.maxGrade !== D.maxGrade || s.surface !== D.surface || s.noRepeat !== D.noRepeat || s.smooth !== D.smooth);
  // mobile : valeur en cours de saisie au pavé (couche `pad` : le bouton Retour annule) ; le pavé reprend la hauteur
  // qu'avait le contenu de la feuille, pour que ni la feuille ni la carte ne bougent
  let editing = $state<Field | null>(null);
  let padH = $state(0);
  const shown = (f: Field) => s.values[type.id][f.param] ?? f.auto?.(s.values[type.id]) ?? f.def ?? f.min;
  function edit(f: Field, e: MouseEvent) {
    const c = (e.currentTarget as HTMLElement).closest<HTMLElement>('.content');
    padH = c ? c.clientHeight - parseFloat(getComputedStyle(c).paddingTop) : 0;
    editing = f;
    openLayer('pad');
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

{#snippet options()}
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
    <span class="label" id="surface-l">{t().surface.label}</span>
    <div class="seg" id="surface" role="radiogroup" aria-labelledby="surface-l" aria-describedby="surface-help">
      {#each ['trail', 'any', 'road'] as const as v}
        <label class:on={s.surface === v}><input type="radio" name="surface" value={v} bind:group={app.settings.surface} />{t().surface[v]}</label>
      {/each}
    </div>
    <p class="note" id="surface-help">{t().surface.help[s.surface]}</p>
  </div>

  <div class="field">
    <label for="loops">{t().more.loops} : <strong>{s.nLoops}</strong></label>
    <input id="loops" type="range" min="1" max="4" step="1" bind:value={app.settings.nLoops} aria-valuetext={String(s.nLoops)} />
  </div>

  {#snippet more()}
    <div class="field">
      <label for="max-grade">{t().more.maxGrade}</label>
      <select id="max-grade" bind:value={app.settings.maxGrade}>
        {#each MAX_GRADES as g}<option value={g}>{g ? `${g} %` : t().more.noLimit}</option>{/each}
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
    <!-- D62 : en bas des Options ; « selon le type » = rien d'envoyé, le moteur applique le défaut du mode -->
    <div class="field">
      <label for="smooth">{t().more.smooth}</label>
      <select id="smooth" bind:value={app.settings.smooth} aria-describedby="smooth-help">
        <option value="auto">{t().more.smoothAuto}</option>
        <option value="on">{t().more.smoothOn}</option>
        <option value="off">{t().more.smoothOff}</option>
      </select>
      <p id="smooth-help" class="note">{t().more.smoothHelp}</p>
    </div>
  {/snippet}
  {#if desktop}
    <details class="more">
      <summary>{t().more.title}</summary>
      {@render more()}
    </details>
  {:else}{@render more()}{/if}
{/snippet}

{#if !desktop && top() === 'pad' && editing}
  {@const f = editing}
  <Keypad field={f} value={s.values[type.id][f.param] ?? null} auto={f.auto ? f.auto(s.values[type.id]) : null} minHeight={padH}
    oncommit={(v) => { app.settings.values[type.id][f.param] = v; void close(); }} oncancel={close} />
{:else if !desktop && top() === 'options'}
  <!-- mobile : page « Options » (couche : bouton Retour du téléphone, flèche, ou « Terminé » sous le pouce) -->
  <header class="page-head">
    <button class="icon-btn" onclick={close} aria-label={t().result.back} title={t().result.back}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.5" /></svg>
    </button>
    <h2>{t().options.title}</h2>
  </header>
  <div class="settings wide-selects">{@render options()}</div>
  <div class="bar"><button class="btn primary wide" onclick={close}>{t().options.done}</button></div>
{:else}
<div class="start-row">
  <Search overlay={!desktop} onpick={(h) => { setStart(h); onfly(h); }} />
  <button class="icon-btn" onclick={myLocation} disabled={locating || computing} aria-label={t().start.myLocation} title={t().start.myLocation}>
    <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><circle cx="12" cy="12" r="4" fill="currentColor" /><circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" stroke-width="2" /><path d="M12 1v4M12 19v4M1 12h4M19 12h4" stroke="currentColor" stroke-width="2" /></svg>
  </button>
</div>
<p class="hint" aria-live="polite">
  {#if locating}{t().start.locating}
  {:else if app.geoError}<span class="err-inline">{t().geo.denied}</span>
  {:else if app.placing}{t().via.hint({ max: String(VIA_MAX) })}
  {:else if startOutsideZone()}<span class="warn-inline">{t().zone.outside}</span>
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
    {#if desktop}
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
    {:else}
      <!-- mobile : chaque valeur est un bouton (toute la case) qui ouvre le pavé numérique intégré -->
      <div class="vals">
        {#each type.fields as f (type.id + f.param)}
          {@const v = s.values[type.id][f.param]}
          {@const name = t().fields[f.param]}
          {@const txt = `${num(i18n.lang, shown(f), 1)} ${f.unit}`}
          <button type="button" class="val" id="f-{f.param}" onclick={(e) => edit(f, e)} aria-label={t().fields.edit({ name, v: v == null ? `auto, ${txt}` : txt })}>
            <span class="vl">{name}</span>
            <span class="vn">{#if v == null}<small>auto</small>{/if}<strong>{num(i18n.lang, shown(f), 1)}</strong> <span class="unit">{f.unit}</span></span>
            <svg class="pen" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20l1-4L16 5l3 3L8 19z" /></svg>
          </button>
        {/each}
      </div>
    {/if}
  {/snippet}
  {#snippet estimate()}
    <span class="estimate" title={t().estimate}><span class="sr">{t().estimate} : </span>
      <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>
      <strong>{est}</strong></span>
  {/snippet}
  <TypePicker value={s.typeId} onselect={setType} fields={typeFields} compact={!desktop} aside={estimate} />
  {#if desktop}{@render options()}{/if}
</fieldset>

{#if desktop}<p class="estimate">{t().estimate} : <strong>{est}</strong></p>{/if}

<div class="bar">
{#if computing}
  <div class="progress-wrap" class:row={!desktop} aria-live="polite">
    <div class="pg">
      <p>{t().compute.computing}</p>
      <div class="progress" class:pulse={pct >= 90} role="progressbar" aria-label={t().compute.computing} aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(pct)}>
        <span style:width="{pct}%"></span>
      </div>
    </div>
    <button class="btn secondary" class:wide={desktop} onclick={cancel}>{t().compute.cancel}</button>
  </div>
{:else}
  <div class="actions" class:icons={!desktop}>
    {#if !desktop}
      <button class="btn secondary ico" onclick={() => openLayer('options')}>
        <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 7h9M17 7h3M4 17h3M11 17h9" /><circle cx="15" cy="7" r="2" /><circle cx="9" cy="17" r="2" /></svg>
        <span class="cap">{t().options.title}</span>{#if optsChanged}<i class="dot"></i>{/if}
      </button>
    {/if}
    <button class="btn secondary" class:ico={!desktop} onclick={() => openLayer('zone')} aria-pressed={!!app.zone}
      aria-label={app.zone ? t().zone.set : t().zone.button} title={app.zone ? t().zone.set : t().zone.button}>
      {#if desktop}{app.zone ? t().zone.set : t().zone.button}{:else}
        <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 6l10-2 5 8-4 8-11-2z" stroke-dasharray="3 3" /></svg>
        <span class="cap">{t().zone.button}</span>{#if app.zone}<i class="dot"></i>{/if}
      {/if}
    </button>
    {#if VIA}
      <button class="btn secondary" class:ico={!desktop} onclick={() => (app.placing = !app.placing)} aria-pressed={app.placing} disabled={!app.start}
        aria-label="{t().via.button}{app.via.length ? ` (${app.via.length})` : ''}" title={t().via.button}>
        {#if desktop}{t().via.button}{app.via.length ? ` (${app.via.length})` : ''}{:else}
          <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="4" cy="12" r="3" fill="currentColor" /><circle cx="20" cy="12" r="3" fill="currentColor" /><path d="M7.600 12h1.600M14.800 12h1.600" /><circle cx="12" cy="12" r="1.700" /></svg>
          <span class="cap">{t().via.short}</span>{#if app.placing || app.via.length}<i class="dot"></i>{/if}
        {/if}
      </button>
    {/if}
    <button class="btn primary grow" onclick={() => compute('new')} disabled={!app.start || app.offline} aria-describedby={app.start ? undefined : 'need-start'}>{t().compute.find}</button>
  </div>
  {#if !app.start}<p id="need-start" class="sr">{t().compute.needStart}</p>{/if}
{/if}
</div>
{/if}

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
  .bar { position: sticky; bottom: 0; z-index: 1; margin-top: auto; background: var(--here, var(--lvl-2)); padding: 8px 16px 12px; border-top: 1px solid var(--border); margin: 0 -16px;
    box-shadow: 0 -6px 12px -8px rgb(var(--sh-rgb) / 0.25); }
  .page-head { display: flex; align-items: center; gap: 10px; margin-bottom: 4px; }
  .page-head h2 { margin: 0; font-size: 1.25rem; }
  .wide-selects select { width: 100%; }
  /* mobile : valeurs côte à côte (la feuille garde la même hauteur d'un type à l'autre) ; chaque case est un bouton */
  .vals { display: flex; gap: 8px; }
  .val { all: unset; box-sizing: border-box; flex: 1 1 0; min-width: 0; max-width: 260px; min-height: 56px; padding: 6px 10px 6px 12px; cursor: pointer;
    display: grid; grid-template-columns: 1fr auto; align-items: center; column-gap: 6px;
    border: 1.5px solid var(--border); border-radius: 12px; background: var(--bg); }
  .val:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
  .val:active { background: var(--accent-soft); }
  .vl { grid-column: 1; font-size: 0.85rem; font-weight: 600; color: var(--muted); line-height: 1.1; }
  .vn { grid-column: 1; white-space: nowrap; font-variant-numeric: tabular-nums; line-height: 1.15; }
  .vn strong { font-size: 1.3rem; }
  .vn small { color: var(--muted); margin-right: 4px; }
  .pen { grid-column: 2; grid-row: 1 / span 2; color: var(--accent-text); }
  span.estimate { margin: 0; flex: none; display: inline-flex; align-items: center; gap: 4px; white-space: nowrap; }
  .via-list { list-style: none; margin: -4px 0 8px; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
  .via-list li { display: flex; align-items: center; gap: 6px; padding-left: 6px; border: 1px solid var(--border); border-radius: 22px; background: var(--surface); }
  .via-dot { display: inline-grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--text); color: var(--bg); font-weight: 800; font-size: 0.85rem; }
  .icon-btn.small { border: 0; background: transparent; border-radius: 50%; }
  .actions { display: flex; gap: 8px; }
  .actions .btn.secondary { padding-inline: 12px; }
  .grow { flex: 1; }
  .actions.icons .grow { padding-inline: 6px; white-space: nowrap; }
  .progress-wrap p { margin: 4px 0 8px; font-weight: 600; }
  /* mobile : même hauteur que la barre d'actions (rien ne bouge au lancement) */
  .progress-wrap.row { display: flex; gap: 12px; align-items: center; min-height: 52px; }
  .row .pg { flex: 1; min-width: 0; } .row p { margin: 0 0 6px; } .row .progress { margin: 0; }
  .progress { height: 10px; border-radius: 5px; background: var(--border); overflow: hidden; margin-bottom: 10px; }
  .progress span { display: block; height: 100%; background: var(--accent); }
  .progress.pulse span { animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.45; } }
  @media (prefers-reduced-motion: reduce) { .progress.pulse span { animation: none; } }
  .err-inline { color: var(--danger); }
  .warn-inline { color: var(--warn-text); }
</style>
