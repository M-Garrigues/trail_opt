// État de l'application et actions (une seule source, partagée par les composants).
import { load, save, remove, clearAll } from './store';
import { mergeSettings, buildQuery, expectedKm, computeEstimateS, settingsFromRequest, type Start } from './settings';
import { byId } from './catalog';
import { postLoop, getLoop, shareUrl, sharedId, type Shared } from './share';
import { fetchPlan, PlanError } from './api';
import { getToken } from './turnstile';
import * as hist from './history';
import { roundStart, inGeometry } from './geo';
import type { Candidate, Msg, PlanResponse } from './types';
import { dicts } from '../i18n/format';
import { i18n } from '../i18n/i18n.svelte';

/** ?debug=1 (ui-spec §4) : versions, durées et codes affichés. */
export const debug = new URLSearchParams(location.search).get('debug') === '1';

export type Layer = 'computing' | 'result' | 'detail' | 'share' | 'history' | 'menu' | 'zone';

export const app = $state({
  settings: mergeSettings(load('settings', null)),
  intro: !load('intro', false),
  start: null as Start | null,
  zone: null as [number, number][] | null, // anneau (lon, lat) validé
  layers: [] as Layer[],
  result: null as PlanResponse | null,
  cands: [] as Candidate[],
  sel: 0,
  seed: 0,
  /** boucle ouverte depuis l'historique ou un lien (pas de recalcul) */
  fromHistory: false,
  /** /b/<id> : boucle partagée en lecture seule */
  shared: false,
  /** paramètres envoyés pour la réponse affichée */
  request: {} as Record<string, string>,
  sharing: false,
  error: null as (Msg & { status?: number }) | null,
  toast: '',
  offline: typeof navigator !== 'undefined' && !navigator.onLine,
  progress: { t0: 0, estS: 1 },
  /** durée du dernier calcul vue du client (s), pour ?debug=1 */
  clientS: 0,
  cursor: -1,
  snap: 1 as 0 | 1 | 2,
  noMore: false,
  geoError: false,
  /** profil affiché dans la bande du bas (bureau, directions nature) */
  dock: false,
});

/** Niveau d'élévation de l'interface (D27) : mobile = cran 1–3 ; bureau = profondeur des couches 1–3. */
export const level = (desktop: boolean) =>
  desktop ? Math.min(3, 1 + app.layers.filter((l) => l !== 'computing').length) : app.snap + 1;
/** Actions carte exposées aux composants (branchées par App). */
export const mapUi = { center: (_i: number) => {} };

export const top = () => app.layers[app.layers.length - 1] ?? null;
export const has = (l: Layer) => app.layers.includes(l);
export const current = () => app.cands[app.sel] ?? null;

$effect.root(() => {
  $effect(() => { save('settings', $state.snapshot(app.settings)); });
});

// ---- couches + bouton Retour (une entrée history par couche, ui-spec §4) ----
const onClose: Partial<Record<Layer, () => void>> = {};
export function onLayerClose(l: Layer, f: () => void) { onClose[l] = f; }

export function open(l: Layer) {
  app.layers.push(l);
  history.pushState({ depth: app.layers.length }, '');
  if (l === 'detail' || l === 'history' || l === 'menu') app.snap = 2;
  else if (l === 'result' || l === 'share') app.snap = Math.max(app.snap, 1) as 1 | 2;
}
/** Remplace la couche du dessus sans nouvelle entrée history. */
function replaceTop(l: Layer) { app.layers[app.layers.length - 1] = l; }
export function close(): Promise<void> { return top() ? closeTo(top()!) : Promise.resolve(); }
/** Ferme jusqu'à la couche l incluse (résolu une fois les couches retirées). */
export function closeTo(l: Layer): Promise<void> {
  const i = app.layers.indexOf(l);
  if (i < 0) return Promise.resolve();
  return new Promise((ok) => {
    addEventListener('popstate', () => setTimeout(ok), { once: true });
    history.go(-(app.layers.length - i));
  });
}
const closeAll = () => (app.layers.length ? closeTo(app.layers[0]) : Promise.resolve());
function popTo(depth: number) {
  while (app.layers.length > depth) {
    const l = app.layers.pop()!;
    onClose[l]?.();
    if (l === 'computing') abort();
    if (l === 'result') clearResult();
  }
  if (!app.layers.length) app.snap = 1;
  else if (top() === 'result') app.snap = 1;
}
addEventListener('popstate', (e) => popTo((e.state as { depth?: number } | null)?.depth ?? 0));

// ---- couverture (web/public/coverage.geojson, lead data) ----
let coverage: GeoJSON.Polygon | GeoJSON.MultiPolygon | null = null;
/** Charge la couverture ; null si absente (l'API reste juge : outside_coverage). */
export async function loadCoverage() {
  try {
    const fc = (await (await fetch('/coverage.geojson')).json()) as GeoJSON.FeatureCollection;
    const g = fc.features.find((f) => f.geometry.type === 'Polygon' || f.geometry.type === 'MultiPolygon')?.geometry;
    coverage = (g as typeof coverage) ?? null;
  } catch { coverage = null; }
  return coverage;
}
export const inCoverage = (p: Start) => !coverage || inGeometry([p.lon, p.lat], coverage);

// ---- départ ----
export function setStart(p: Start) {
  app.start = p;
  app.geoError = false;
  if (has('result')) void closeTo('result');
}

export function locate() {
  return new Promise<Start | null>((ok) => {
    if (!navigator.geolocation) { app.geoError = true; return ok(null); }
    navigator.geolocation.getCurrentPosition(
      (pos) => { const p = { lat: pos.coords.latitude, lon: pos.coords.longitude }; setStart(p); ok(p); },
      () => { app.geoError = true; ok(null); },
      { enableHighAccuracy: true, timeout: 15000, maximumAge: 60000 },
    );
  });
}

export function forgetPosition() { remove('lastStart'); }
export function eraseAll() { clearAll(); location.reload(); }

// ---- calcul ----
let ctrl: AbortController | null = null;
function abort() { ctrl?.abort(); ctrl = null; }
export function cancel() { if (has('computing')) void closeTo('computing'); }

function clearResult() {
  app.result = null; app.cands = []; app.sel = 0; app.cursor = -1; app.fromHistory = false; app.noMore = false;
  if (app.shared) { app.shared = false; history.replaceState(history.state, '', '/' + location.search); }
}

export function toast(msg: string) {
  app.toast = msg;
  setTimeout(() => { if (app.toast === msg) app.toast = ''; }, 4000);
}

/** mode 'new' : calcul ; 'more' : « Autres boucles » (n=4, même graine) ; 'seed' : nouvelles propositions. */
export async function compute(mode: 'new' | 'more' | 'seed' = 'new') {
  if (!app.start || has('computing')) return;
  if (app.offline) { app.error = { code: 'offline' }; return; }
  if (!inCoverage(app.start)) { app.error = { code: 'outside_coverage', params: {} }; return; }
  app.error = null;
  if (mode === 'new') app.seed = 0;
  if (mode === 'seed') app.seed = (app.seed + 1) % 1000;
  const n = mode === 'more' ? 4 : app.settings.nLoops;
  const q = buildQuery(app.settings, app.start, { n, seed: app.seed, polygon: app.zone });
  const keep = mode === 'more' ? current() : null;
  if (mode !== 'more') await closeAll();
  ctrl = new AbortController();
  const my = ctrl;
  app.progress = { t0: performance.now(), estS: computeEstimateS(expectedKm(app.settings), byId(app.settings.typeId).goal, n) };
  open('computing');
  try {
    const token = await getToken();
    if (my.signal.aborted) return;
    const res = await fetchPlan(q, token, my.signal);
    if (my.signal.aborted) return;
    let cands = res.candidates;
    if (keep) {
      // api.md v1.1 § Déterminisme : retirer le candidat identique (lat/lon égaux) ; aucun → les 4 sans retrait
      const same = (c: Candidate) => c.lat.length === keep.lat.length && c.lat.every((v, i) => v === keep.lat[i] && c.lon[i] === keep.lon[i]);
      // la copie du serveur plutôt que keep : sa `sig` couvre les avertissements et la zone de CETTE réponse
      const kept = cands.find(same);
      cands = kept ? [kept, ...cands.filter((c) => c !== kept)] : cands;
      app.noMore = cands.length < 2;
      app.sel = 0;
      ctrl = null;
      await closeTo('computing');
      app.cands = cands;
      app.result = { ...res, candidates: cands };
    } else {
      replaceTop('result');
      app.result = res; app.cands = cands; app.sel = 0; app.noMore = false; app.fromHistory = false;
    }
    app.request = Object.fromEntries(q);
    app.clientS = (performance.now() - app.progress.t0) / 1000;
    app.snap = 1;
    ctrl = null;
    save('lastStart', roundStart(app.start!));
    const d = dicts[i18n.lang];
    for (const c of keep ? cands.slice(1) : cands) {
      const r = hist.add({ settings: $state.snapshot(app.settings), start: { ...app.start! }, candidate: c, warnings: res.warnings,
        request: Object.fromEntries(q), effective_start: res.effective_start,
        lower_bound_m: res.lower_bound_m, data_version: res.data_version, solver_version: res.solver_version });
      if (r.status === 'full') { toast(d.history.full); break; }
      if (r.status === 'unavailable') { if (!warnedStorage) toast(d.history.unavailable); warnedStorage = true; break; }
    }
  } catch (e) {
    if (my.signal.aborted) return;
    ctrl = null;
    const pe = e instanceof PlanError ? e : new PlanError('network');
    if (pe.code === 'cancelled') return;
    if (has('computing')) await closeTo('computing');
    app.error = { code: pe.code, params: pe.params ?? {}, status: pe.status };
  }
}
let warnedStorage = false;

export function select(i: number) {
  if (i >= 0 && i < app.cands.length) { app.sel = i; app.cursor = -1; }
}

export async function openEntry(e: hist.Entry) {
  await closeAll();
  app.settings = mergeSettings(e.settings);
  app.start = e.start;
  const res: PlanResponse = {
    solver_version: e.solver_version, data_version: e.data_version, compute_s: 0, lower_bound_m: e.lower_bound_m,
    effective_start: e.effective_start ?? { lat: e.candidate.lat[0], lon: e.candidate.lon[0], kind: 'clicked' }, zone: null,
    candidates: [e.candidate], warnings: e.warnings,
  };
  open('result');
  app.result = res; app.cands = [e.candidate]; app.sel = 0; app.fromHistory = true; app.noMore = false;
  app.request = e.request ?? {};
  app.snap = 1;
}

export function dismissIntro() { app.intro = false; save('intro', true); }

// ---- partage (E9, E10) ----
/** Crée le lien (jeton Turnstile NEUF) puis navigator.share, sinon copie. Renvoie l'URL ou null. */
export async function shareLoop(): Promise<string | null> {
  const c = current(), r = app.result;
  if (!c || !r || app.sharing) return null;
  app.sharing = true;
  app.error = null;
  try {
    // Boucle ouverte par lien : on repartage son URL (GET ne rend pas `sig`, api.md v1.3).
    // Sinon tout est recopié tel quel de la même réponse /api/plan : `sig` les couvre.
    const body: Shared = {
      request: app.request, data_version: r.data_version, solver_version: r.solver_version,
      effective_start: r.effective_start, zone: r.zone ?? null, candidate: $state.snapshot(c) as Candidate, warnings: r.warnings,
    };
    const id = (app.shared && sharedId()) || (await postLoop(body, await getToken()));
    const url = shareUrl(id);
    const d = dicts[i18n.lang];
    try {
      if (navigator.share) await navigator.share({ url, title: d.share.title });
      else { await navigator.clipboard.writeText(url); toast(d.share.copied); }
    } catch (e) {
      if ((e as Error)?.name !== 'AbortError') { try { await navigator.clipboard.writeText(url); toast(d.share.copied); } catch { /* rien */ } }
    }
    return url;
  } catch (e) {
    const pe = e instanceof PlanError ? e : new PlanError('network');
    app.error = { code: pe.code, params: pe.params ?? {}, status: pe.status };
    return null;
  } finally {
    app.sharing = false;
  }
}

/** /b/<id> : ouvre la boucle partagée sans appel /api/plan. */
export async function openShared(id: string) {
  try {
    const s = await getLoop(id);
    app.settings = settingsFromRequest(s.request, app.settings);
    const lat = +s.request.lat, lon = +s.request.lon;
    app.start = Number.isFinite(lat) && Number.isFinite(lon) ? { lat, lon } : { lat: s.effective_start.lat, lon: s.effective_start.lon };
    open('result');
    app.result = { solver_version: s.solver_version, data_version: s.data_version, compute_s: 0, lower_bound_m: null,
      effective_start: s.effective_start, zone: s.zone ?? null, candidates: [s.candidate], warnings: s.warnings };
    app.cands = [s.candidate]; app.sel = 0; app.fromHistory = true; app.shared = true; app.request = s.request;
    app.snap = 1;
  } catch (e) {
    const pe = e instanceof PlanError ? e : new PlanError('network');
    app.error = { code: pe.code, params: pe.params ?? {}, status: pe.status };
    history.replaceState(null, '', '/' + location.search);
  }
}
