<script lang="ts">
  import { onMount } from 'svelte';
  import { app, top, has, current, compute, close, cancel, setStart, select, open as openLayer, dismissIntro, openShared, mapUi, level, loadCoverage, debug } from './lib/app.svelte';
  import { nature, wave, wavePath } from './lib/design';
  import Profile from './components/Profile.svelte';
  import { sharedId } from './lib/share';
  import { TrailMap } from './lib/map';
  import { nearestIndex } from './lib/geo';
  import { load } from './lib/store';
  import { REJECTED, errorText } from './i18n/format';
  import { i18n, t, setLang } from './i18n/i18n.svelte';
  import Sheet from './components/Sheet.svelte';
  import Planner from './components/Planner.svelte';
  import Result from './components/Result.svelte';
  import Detail from './components/Detail.svelte';
  import Share from './components/Share.svelte';
  import History from './components/History.svelte';
  import Menu from './components/Menu.svelte';

  let mapEl: HTMLDivElement;
  let tmap = $state<TrailMap | null>(null);
  let desktop = $state(matchMedia('(min-width: 1024px)').matches);
  const layer = $derived(top());
  // D27 : bureau + direction nature → profil dans une bande en bas, fondue avec le panneau
  const DOCK_H = 168;
  $effect(() => { app.dock = nature && desktop && !!app.result && !!current(); });
  let view3d = $state(false);
  const padding = () => desktop
    ? { top: 60, bottom: 40 + (app.dock ? DOCK_H : 0), left: 60, right: 60 }
    : { top: 60, bottom: Math.round(innerHeight * 0.5) + 20, left: 30, right: 30 };
  const cap = new URLSearchParams(location.search).get('cap') === 'km1' ? 'km1' : 'centroid';
  function toggle3d() {
    view3d = !view3d;
    void tmap?.set3D(view3d, current(), padding(), cap);
  }
  // clic/appui sur le profil → centre la carte sur ce point, visible au-dessus de la feuille / de la bande
  mapUi.center = (i) => {
    const c = current();
    if (!c || !tmap) return;
    const sheet = document.querySelector('.sheet') as HTMLElement | null;
    tmap.centerOn(c, i, desktop ? (app.dock ? DOCK_H / 2 : 0) : (sheet?.offsetHeight ?? 0) / 2);
  };
  const dockEdge = (dy: number) => wavePath(wave(1000, 14, 7, 11, dy));

  onMount(() => {
    const mq = matchMedia('(min-width: 1024px)');
    const onMq = () => (desktop = mq.matches);
    mq.addEventListener('change', onMq);
    const last = load<{ lat: number; lon: number } | null>('lastStart', null);
    tmap = new TrailMap(mapEl, {
      onClick: (p) => { if (!has('result') && !has('computing')) setStart(p); },
      onLoopClick: (i) => { if (has('result')) select(i); },
      onLoopHover: (i, ll) => {
        const c = current();
        app.cursor = c && ll && i === app.sel ? nearestIndex(c.lat, c.lon, ll) : -1;
      },
      onStartDrag: (p) => setStart(p),
    }, last ?? undefined, t().start.marker);
    if (import.meta.env.DEV) (window as unknown as { tmap: TrailMap }).tmap = tmap;
    const map = tmap;
    void loadCoverage().then((g) => { if (g) void map.setCoverage(g, !last && !sharedId() && !app.start); });
    const sid = sharedId();
    if (sid) { app.intro = false; void openShared(sid); }
    const on = () => (app.offline = false), off = () => (app.offline = true);
    addEventListener('online', on);
    addEventListener('offline', off);
    return () => { mq.removeEventListener('change', onMq); removeEventListener('online', on); removeEventListener('offline', off); };
  });

  // carte ← état
  $effect(() => { tmap?.setStart(app.start); });
  $effect(() => { tmap?.setLoops(app.cands, app.sel); });
  $effect(() => { tmap?.setCursor(current(), app.cursor); });
  $effect(() => {
    const z = app.result?.zone;
    const used = z && (app.zone || z.reduced_radius_km) ? z.geometry : null;
    const own: GeoJSON.Geometry | null = app.zone ? { type: 'Polygon', coordinates: [[...app.zone, app.zone[0]]] } : null;
    if (layer !== 'zone') tmap?.setZone(used ?? own);
    else tmap?.setZone(null);
  });
  // cadrage sur la boucle à chaque nouveau résultat
  $effect(() => {
    const r = app.result;
    if (!r || !tmap || !r.candidates[0]) return;
    if (view3d) void tmap.set3D(true, r.candidates[0], padding(), cap);
    else tmap.fitLoop(r.candidates[0], padding());
  });
  $effect(() => { document.title = `optrail · ${t().app.tagline}`; });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (app.error) { app.error = null; return; }
      if (has('computing')) { cancel(); return; }
      if (layer) void close();
    } else if (e.key === 'Enter' && !layer && !app.intro) {
      const el = e.target as HTMLElement;
      if (el.tagName === 'INPUT' && (el as HTMLInputElement).type === 'number') {
        el.blur();
        setTimeout(() => compute('new'));
      }
    }
  }

  // E13 : action proposée par code
  const errAction = $derived.by(() => {
    const c = app.error?.code;
    if (!c) return null;
    const a = t().actions;
    if (c === 'offline' || c === 'service_paused') return null;
    if (c === 'outside_coverage') return { label: a.seeCoverage, run: () => tmap?.fitCoverage() };
    if (c === 'no_way_in_zone') return { label: a.widenRoads, run: () => { app.settings.roads = 'all'; } };
    if (c === 'no_loop_of_distance' || c === 'distance_out_of_range' || c === 'dplus_out_of_range' || c === 'dplus_unreachable_proven')
      return { label: a.changeSettings, run: () => { app.snap = 1; document.querySelector<HTMLInputElement>('.field input[type=number]')?.focus(); } };
    if (c === 'zone_invalid') return { label: a.redraw, run: () => { app.zone = null; openLayer('zone'); } };
    if (c === 'start_outside_zone' || c === 'zone_too_large') return { label: a.editZone, run: () => openLayer('zone') };
    if (REJECTED.has(c)) return { label: a.reload, run: () => location.reload() };
    return { label: a.retry, run: () => compute('new') };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="app" class:desktop class:docked={app.dock} style:--dock-h="{app.dock ? DOCK_H : 0}px">
  <div class="map" bind:this={mapEl} role="application" aria-label="Carte / Map"></div>

  <header class="topbar">
    <span class="brand">optrail</span>
    <div class="lang" role="radiogroup" aria-label={t().app.langLabel}>
      <button role="radio" aria-checked={i18n.lang === 'fr'} onclick={() => setLang('fr')} lang="fr">FR</button>
      <span aria-hidden="true">·</span>
      <button role="radio" aria-checked={i18n.lang === 'en'} onclick={() => setLang('en')} lang="en">EN</button>
    </div>
    <button class="icon-btn" onclick={() => openLayer('history')} aria-label={t().history.title} title={t().history.title}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M3 12a9 9 0 1 0 3-6.7M3 4v5h5M12 7v5l3 3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
    </button>
    <button class="icon-btn" onclick={() => openLayer('menu')} aria-label={t().menu.open} title={t().menu.open}>
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
    </button>
  </header>

  {#if app.shared && layer === 'result'}<div class="shared-banner" role="status">{t().share.banner}</div>{/if}
  {#if app.offline}<div class="banner" role="status">{t().offline}</div>{/if}

  {#if app.error && app.error.code !== 'cancelled'}
    <div class="alert" role="alert">
      <p>{errorText(i18n.lang, app.error.code, app.error.params ?? {})}</p>
      {#if debug}<p class="debug" data-testid="debug-error">{app.error.code}{app.error.status ? ` · HTTP ${app.error.status}` : ''}</p>{/if}
      <div class="alert-actions">
        {#if errAction}<button class="btn primary small" onclick={() => { const r = errAction.run; app.error = null; r(); }}>{errAction.label}</button>{/if}
        <button class="btn secondary small" onclick={() => (app.error = null)}>{t().actions.close}</button>
      </div>
    </div>
  {/if}

  {#if nature}
    <button class="icon-btn view3d" onclick={toggle3d} aria-pressed={view3d} aria-label={view3d ? t().detail.view2d : t().detail.view3d}
      title={view3d ? t().detail.view2d : t().detail.view3d}>{view3d ? '2D' : '3D'}</button>
  {/if}

  {#if app.dock && current()}
    {@const lv = level(desktop)}
    <section class="dock" data-level={lv} aria-label={t().detail.legend}>
      <svg class="dock-edge" viewBox="0 0 1000 40" preserveAspectRatio="none" aria-hidden="true">
        {#each [3, 2, 1] as k}<path class="ring" class:on={k <= lv} d={dockEdge(6 * k)} />{/each}
        <path class="fill" d="{dockEdge(0)}L1000,40L0,40Z" />
      </svg>
      <div class="topo dock-topo"></div>
      <div class="dock-in">
        <Profile cand={current()!} bind:cursor={app.cursor} height={DOCK_H - 44} onpick={mapUi.center} />
        <p class="dock-hint">{t().detail.centerHint}</p>
      </div>
    </section>
  {/if}

  {#if app.toast}<div class="toast" role="status">{app.toast}</div>{/if}

  <Sheet {desktop} label={t().app.tagline}>
    {#if app.intro}
      <h1>{t().app.tagline}</h1>
      <p>{t().intro.text}</p>
      <p class="muted">{t().intro.coverage}</p>
      <p class="safety" data-testid="safety">{t().safety}</p>
      <button class="btn primary wide" onclick={dismissIntro}>{t().intro.go}</button>
    {:else if layer === 'zone' && tmap}
      {#await import('./components/Zone.svelte') then Z}<Z.default {tmap} />{/await}
    {:else if layer === 'menu'}
      <Menu />
    {:else if layer === 'history'}
      <History />
    {:else if layer === 'share'}
      <Share />
    {:else if layer === 'detail'}
      <Detail />
    {:else if layer === 'result'}
      <Result />
    {:else}
      <Planner onfly={(p) => tmap?.flyTo(p, 14, desktop ? 200 : 0, desktop ? 0 : innerHeight * 0.25)} />
    {/if}
  </Sheet>
</div>

<style>
  .app { position: fixed; inset: 0; overflow: hidden; }
  /* carte toujours claire (D24) : fond clair tant que les tuiles chargent */
  .map { position: absolute; inset: 0; background: #f2efe9; }
  .app.desktop .map { left: 400px; }
  .topbar {
    position: absolute; top: calc(8px + env(safe-area-inset-top)); left: 8px; right: 56px; z-index: 6; display: flex; gap: 6px;
    align-items: center; pointer-events: none;
  }
  .app.desktop .topbar { left: 408px; }
  .topbar > * { pointer-events: auto; }
  .brand {
    font-weight: 800; font-size: 1.15rem; background: var(--bg); color: var(--text); padding: 8px 12px; border-radius: 10px;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); margin-right: auto;
  }
  .lang { display: flex; align-items: center; background: var(--bg); color: var(--text); border-radius: 10px; box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); }
  .lang button { all: unset; min-width: 40px; min-height: 44px; text-align: center; cursor: pointer; font-weight: 600; color: var(--muted); }
  .lang button[aria-checked='true'] { color: var(--accent-text); text-decoration: underline; text-underline-offset: 4px; }
  .lang button:focus-visible { outline: 3px solid var(--accent); }
  .topbar .icon-btn { background: var(--bg); box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); }
  .banner {
    position: absolute; top: calc(64px + env(safe-area-inset-top)); left: 8px; right: 8px; z-index: 7; background: var(--danger);
    color: #fff; padding: 10px 12px; border-radius: 10px; font-weight: 600;
  }
  .app.desktop .banner, .app.desktop .alert, .app.desktop .toast { left: 416px; right: auto; width: 420px; }
  .shared-banner {
    position: absolute; top: calc(64px + env(safe-area-inset-top)); left: 8px; z-index: 6; background: var(--accent); color: var(--on-accent);
    padding: 8px 12px; border-radius: 10px; font-weight: 700;
  }
  .app.desktop .shared-banner { left: 416px; }
  .alert {
    position: absolute; top: calc(64px + env(safe-area-inset-top)); left: 8px; right: 8px; z-index: 8; background: var(--surface);
    color: var(--text); border-left: 6px solid var(--danger); padding: 10px 12px; border-radius: 10px; box-shadow: 0 2px 10px rgb(0 0 0 / 0.3);
  }
  .alert p { margin: 0 0 8px; font-weight: 600; }
  .alert .debug { font: 0.8rem ui-monospace, monospace; color: var(--muted); }
  .alert-actions { display: flex; gap: 8px; }
  .toast {
    position: absolute; top: calc(64px + env(safe-area-inset-top)); left: 8px; right: 8px; z-index: 9; background: var(--text);
    color: var(--bg); padding: 10px 12px; border-radius: 10px;
  }
  h1 { font-size: 1.4rem; margin: 4px 0 8px; }
  .view3d { position: absolute; z-index: 4; right: 10px; top: calc(108px + env(safe-area-inset-top)); background: var(--bg); font-weight: 800;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); }
  .view3d[aria-pressed='true'] { background: var(--accent); color: var(--on-accent); border-color: var(--accent); }
  .dock { position: absolute; left: 400px; right: 0; bottom: 0; height: var(--dock-h); z-index: 4; background: var(--lvl-2);
    box-shadow: 0 -3px 12px rgb(0 0 0 / 0.18); transition: background-color 0.4s; }
  .dock[data-level='3'] { background: var(--lvl-3); }
  .dock-edge { position: absolute; left: 0; right: 0; bottom: calc(100% - 14px); width: 100%; height: 40px; overflow: visible; pointer-events: none; }
  .dock-edge .fill { fill: var(--lvl-2); }
  .dock[data-level='3'] .dock-edge .fill { fill: var(--lvl-3); }
  .dock-edge .ring { fill: none; stroke: var(--contour); stroke-width: 1.3; vector-effect: non-scaling-stroke; opacity: 0; transition: opacity 0.5s; }
  .dock-edge .ring.on { opacity: 0.7; }
  .dock-topo { -webkit-mask-image: var(--topo), linear-gradient(#000, transparent 70%); mask-image: var(--topo), linear-gradient(#000, transparent 70%);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: 480px, 100% 100%; mask-size: 480px, 100% 100%; }
  .dock-in { position: relative; padding: 6px 24px 0 40px; }
  .dock-hint { margin: 0; font-size: 0.8rem; color: var(--muted); }
  .app.docked :global(.maplibregl-ctrl-bottom-right), .app.docked :global(.maplibregl-ctrl-bottom-left) { bottom: var(--dock-h); }
  @media (prefers-reduced-motion: reduce) { .dock, .dock-edge .ring { transition: none; } }
  .safety { background: var(--warn-bg); color: var(--text); border-radius: 10px; padding: 10px 12px; line-height: 1.45; }
  .muted { color: var(--muted); }
</style>
