import { byId, defaults, CATALOG, type TypeId, type Values } from './catalog';

export type Climbs = 'short' | 'balanced' | 'long';
/** Type de voie préféré (api.md v1.7) : jamais un filtre. */
export type Surface = 'trail' | 'any' | 'road';
const isSurface = (x: unknown): x is Surface => x === 'trail' || x === 'any' || x === 'road';
/** D62 « Sorties plus fluides » : `auto` = défaut du mode (rien n'est envoyé), sinon forcé. */
export type Smooth = 'auto' | 'on' | 'off';
const isSmooth = (x: unknown): x is Smooth => x === 'auto' || x === 'on' || x === 'off';
export type Settings = {
  typeId: TypeId;
  values: Record<TypeId, Values>;
  climbs: Climbs;
  /** % sur 50 m glissants (api.md v1.2) ; 0 = sans limite */
  maxGrade: number;
  surface: Surface;
  noRepeat: boolean;
  smooth: Smooth;
  paceS: number; // s par km-effort
  nLoops: number;
};

export const MAX_GRADE_DEFAULT = 60;
/** Choix proposés : 5–60 % par pas de 5, puis 0 = sans limite (api.md v1.2). */
export const MAX_GRADES = [...Array.from({ length: 12 }, (_, i) => 5 + i * 5), 0];
const okGrade = (g: unknown) => typeof g === 'number' && MAX_GRADES.includes(g);

export function defaultSettings(): Settings {
  return {
    typeId: 'target',
    values: Object.fromEntries(CATALOG.map((t) => [t.id, defaults(t)])) as Record<TypeId, Values>,
    climbs: 'balanced',
    maxGrade: MAX_GRADE_DEFAULT,
    surface: 'trail',
    noRepeat: true,
    smooth: 'auto',
    paceS: 360,
    nLoops: 1,
  };
}

/** Fusionne un objet mémorisé (peut-être ancien ou corrompu) avec les défauts. */
export function mergeSettings(saved: unknown): Settings {
  const d = defaultSettings();
  if (!saved || typeof saved !== 'object') return d;
  const s = { ...d, ...(saved as Partial<Settings>) };
  s.typeId = byId(s.typeId).id;
  s.values = { ...d.values };
  const sv = (saved as Partial<Settings>).values ?? {};
  for (const t of CATALOG) s.values[t.id] = { ...d.values[t.id], ...((sv as Record<string, Values>)[t.id] ?? {}) };
  // avant v1.2 : { maxGradeOn: false } = pas de filtre → nouveau défaut 60 %
  const old = saved as { maxGradeOn?: boolean };
  if (old.maxGradeOn === false || !okGrade(s.maxGrade)) s.maxGrade = MAX_GRADE_DEFAULT;
  delete (s as { maxGradeOn?: boolean }).maxGradeOn;
  if (!isSmooth(s.smooth)) s.smooth = 'auto';
  // avant v1.7 : filtre `roads` ; seul « toutes routes » (choisi exprès) devient « indifférent »
  const roads = (s as { roads?: string }).roads;
  if (!isSurface((saved as Partial<Settings>).surface)) s.surface = roads === 'all' ? 'any' : d.surface;
  delete (s as { roads?: string }).roads;
  return s;
}

export type Start = { lat: number; lon: number };

/** Paramètres de GET /api/plan (api.md v1), rien d'autre. */
export function buildQuery(s: Settings, start: Start, opts: { n: number; seed: number; polygon?: [number, number][] | null; via?: Start[] }) {
  const t = byId(s.typeId);
  const v = s.values[t.id];
  const q = new URLSearchParams();
  q.set('lat', start.lat.toFixed(6));
  q.set('lon', start.lon.toFixed(6));
  q.set('goal', t.goal);
  if (t.goal !== 'min_distance') q.set('distance_km', String(v.distance_km ?? 10));
  if (t.goal !== 'max_dplus') q.set('dplus_m', String(v.dplus_m));
  if (t.goal === 'min_distance' && v.max_distance_km != null) q.set('max_distance_km', String(v.max_distance_km));
  q.set('climbs', s.climbs);
  if (s.maxGrade !== MAX_GRADE_DEFAULT) q.set('max_grade_pct', String(s.maxGrade)); // absent = 60 (api.md v1.2)
  q.set('surface', s.surface);
  q.set('no_repeat_junction', String(s.noRepeat));
  if (s.smooth !== 'auto') q.set('smooth', String(s.smooth === 'on')); // D62 : absent = défaut du mode
  q.set('n_candidates', String(opts.n));
  if (opts.polygon?.length) q.set('polygon', opts.polygon.map(([lo, la]) => `${lo.toFixed(5)},${la.toFixed(5)}`).join(';'));
  if (opts.via?.length) q.set('via', opts.via.map((p) => `${p.lat.toFixed(6)},${p.lon.toFixed(6)}`).join(';'));
  q.set('seed', String(opts.seed));
  return q;
}

/** Type de voie d'une requête (liens partagés, historique) : `surface`, sinon l'ancien `roads` traduit comme le fait l'API
 *  et la migration des réglages (seul `all` devient `any`) ; null si absent. */
export function requestSurface(req: Record<string, string>): Surface | null {
  if (isSurface(req.surface)) return req.surface;
  if (req.roads === 'unpaved' || req.roads === 'pedestrian' || req.roads === 'minor') return 'trail';
  if (req.roads === 'all') return 'any';
  return null;
}

/** Part (0–1) de chemin d'une sortie en « Chemins », si elle est sous 50 % (avertissement) ; sinon null.
 *  Jamais en « Le plus court » (la préférence n'y agit pas) ni en « Route » (D53 : tout est revêtu). */
export function lowSurface(c: { trail_frac?: number }, req: Record<string, string>): { share: number } | null {
  if (c.trail_frac == null || requestSurface(req) !== 'trail' || req.goal === 'min_distance') return null;
  return c.trail_frac < 0.5 ? { share: c.trail_frac } : null;
}

/** Réglages préremplis depuis la requête d'une boucle partagée (« Recalculer depuis ici »). */
export function settingsFromRequest(req: Record<string, string>, base: Settings): Settings {
  const s = mergeSettings(base);
  const t = CATALOG.find((c) => c.goal === (req.goal ?? 'max_dplus'))!; // goal absent = max_dplus (défaut de l'API)
  s.typeId = t.id;
  for (const f of t.fields) if (req[f.param] != null && Number.isFinite(+req[f.param])) s.values[t.id][f.param] = +req[f.param];
  if (req.climbs === 'short' || req.climbs === 'balanced' || req.climbs === 'long') s.climbs = req.climbs;
  s.surface = requestSurface(req) ?? s.surface;
  s.maxGrade = req.max_grade_pct == null ? MAX_GRADE_DEFAULT : okGrade(+req.max_grade_pct) ? +req.max_grade_pct : s.maxGrade;
  if (req.no_repeat_junction) s.noRepeat = req.no_repeat_junction === 'true' || req.no_repeat_junction === '1';
  s.smooth = req.smooth == null ? 'auto' : req.smooth === 'true' || req.smooth === '1' ? 'on' : 'off';
  return s;
}

/** Distance (km) attendue avant calcul, pour la durée estimée et la barre. */
export function expectedKm(s: Settings): number {
  const t = byId(s.typeId);
  const v = s.values[t.id];
  if (t.goal === 'min_distance') return v.max_distance_km ?? t.fields[1].auto!(v);
  return v.distance_km ?? 10;
}

/** Durée (min) = (km + D+/100) × allure. */
export const durationMin = (km: number, dplus: number, paceS: number) => ((km + dplus / 100) * paceS) / 60;

/** Estimation du temps de calcul (s), recalée sur la Lambda réelle (scripts/lambda_bench.sh, 2026-10-06 ;
 *  api.md § Durée estimée) : n ≤ 2 au-delà de 40 km ; « longues » ×1,8 sauf en cible, où la préférence de
 *  montées n'agit plus sur la recherche ; « le plus court » ×0,6 ; +0,5 s de réseau. */
export const computeEstimateS = (D: number, goal: string, n: number, climbs: string = 'balanced') => {
  const k = D > 40 ? Math.min(n, 2) : n;
  const f = (goal === 'min_distance' ? 0.6 : 1) * (climbs === 'long' && goal !== 'target' ? 1.8 : 1);
  return Math.min(15, (3.2 + 0.085 * D) * (1 + 0.12 * (k - 1)) * f) + 0.5;
};
