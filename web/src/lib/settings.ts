import { byId, defaults, fieldsOf, profile, CATALOG, SURFACES, SURFACE_DEFAULT, type Sport, type Surface, type TypeId, type Values } from './catalog';

export type Climbs = 'short' | 'balanced' | 'long';
/** Type de voie (api.md v1.7, § Sports) : préférence en trail, jamais un filtre sauf `road`. */
const isSurface = (x: unknown): x is Surface => x === 'trail' || x === 'any' || x === 'road' || x === 'mtb' || x === 'gravel';
/** D62 « Sorties plus fluides » : `auto` = défaut du mode (rien n'est envoyé), sinon forcé. */
export type Smooth = 'auto' | 'on' | 'off';
const isSmooth = (x: unknown): x is Smooth => x === 'auto' || x === 'on' || x === 'off';
export type Settings = {
  typeId: TypeId;
  values: Record<TypeId, Values>;
  climbs: Climbs;
  /** % sur 50 m glissants (api.md v1.2) ; 0 = sans limite (trail) ; vélo : la montée (D73) */
  maxGrade: number;
  surface: Surface;
  noRepeat: boolean;
  smooth: Smooth;
  paceS: number; // s par km-effort
  nLoops: number;
};

/** Pente max : défaut du moteur et choix proposés selon le profil (trail 5–60 par 5 + sans limite ; vélo 5–40). */
export const gradeDefault = (sport: Sport, surface: string) => profile(sport, surface).grade;
export const gradeChoices = (sport: Sport, surface: string) => profile(sport, surface).grades;
const okGrade = (sport: Sport, surface: string, g: unknown) => typeof g === 'number' && gradeChoices(sport, surface).includes(g);

export function defaultSettings(sport: Sport = 'trail'): Settings {
  const surface = SURFACE_DEFAULT[sport];
  return {
    typeId: 'target',
    values: Object.fromEntries(CATALOG.map((t) => [t.id, defaults(fieldsOf(t, sport, surface))])) as Record<TypeId, Values>,
    climbs: 'balanced',
    maxGrade: gradeDefault(sport, surface),
    surface,
    noRepeat: true,
    smooth: 'auto',
    paceS: 360,
    nLoops: 1,
  };
}

/** Fusionne un objet mémorisé (peut-être ancien ou corrompu) avec les défauts du sport. */
export function mergeSettings(saved: unknown, sport: Sport = 'trail'): Settings {
  const d = defaultSettings(sport);
  if (!saved || typeof saved !== 'object') return d;
  const s = { ...d, ...(saved as Partial<Settings>) };
  s.typeId = byId(s.typeId).id;
  s.values = { ...d.values };
  const sv = (saved as Partial<Settings>).values ?? {};
  for (const t of CATALOG) s.values[t.id] = { ...d.values[t.id], ...((sv as Record<string, Values>)[t.id] ?? {}) };
  // avant v1.2 : { maxGradeOn: false } = pas de filtre → nouveau défaut 60 %
  const old = saved as { maxGradeOn?: boolean };
  if (!SURFACES[sport].includes(s.surface)) s.surface = SURFACE_DEFAULT[sport]; // réglages d'un autre sport ou corrompus
  if (old.maxGradeOn === false || !okGrade(sport, s.surface, s.maxGrade)) s.maxGrade = gradeDefault(sport, s.surface);
  delete (s as { maxGradeOn?: boolean }).maxGradeOn;
  if (!isSmooth(s.smooth)) s.smooth = 'auto';
  // avant v1.7 : filtre `roads` ; seul « toutes routes » (choisi exprès) devient « indifférent »
  const roads = (s as { roads?: string }).roads;
  if (!isSurface((saved as Partial<Settings>).surface)) s.surface = roads === 'all' && sport === 'trail' ? 'any' : d.surface;
  delete (s as { roads?: string }).roads;
  return s;
}

/** Change le type de voie : une distance ou une pente restée au défaut de l'ancienne voie prend celui de la nouvelle
 *  (VTT 30 km → gravel 50 km), une valeur choisie est seulement ramenée dans les nouvelles bornes. */
export function setSurface(s: Settings, sport: Sport, next: Surface) {
  const a = profile(sport, s.surface), b = profile(sport, next);
  for (const t of CATALOG) {
    const v = s.values[t.id];
    if (v.distance_km != null) v.distance_km = v.distance_km === a.dist.def ? b.dist.def : Math.min(b.dist.max, Math.max(b.dist.min, v.distance_km));
  }
  if (s.maxGrade === a.grade) s.maxGrade = b.grade;
  s.surface = next;
}

export type Start = { lat: number; lon: number };

/** Paramètres de GET /api/plan (api.md v1), rien d'autre. `sport=bike` seulement en vélo (absent = trail). */
export function buildQuery(s: Settings, start: Start, opts: { n: number; seed: number; polygon?: [number, number][] | null; via?: Start[]; sport?: Sport }) {
  const t = byId(s.typeId);
  const v = s.values[t.id];
  const sport = opts.sport ?? 'trail';
  const q = new URLSearchParams();
  q.set('lat', start.lat.toFixed(6));
  q.set('lon', start.lon.toFixed(6));
  if (sport === 'bike') q.set('sport', 'bike');
  q.set('goal', t.goal);
  if (t.goal !== 'min_distance') q.set('distance_km', String(v.distance_km ?? 10));
  if (t.goal !== 'max_dplus') q.set('dplus_m', String(v.dplus_m));
  if (t.goal === 'min_distance' && v.max_distance_km != null) q.set('max_distance_km', String(v.max_distance_km));
  q.set('climbs', s.climbs);
  if (s.maxGrade !== gradeDefault(sport, s.surface)) q.set('max_grade_pct', String(s.maxGrade)); // absent = défaut du profil (60 trail, 22/20/20 vélo)
  q.set('surface', s.surface);
  q.set('no_repeat_junction', String(s.noRepeat));
  if (s.smooth !== 'auto') q.set('smooth', String(s.smooth === 'on')); // D62 : absent = défaut du mode
  q.set('n_candidates', String(opts.n));
  if (opts.polygon?.length) q.set('polygon', opts.polygon.map(([lo, la]) => `${lo.toFixed(5)},${la.toFixed(5)}`).join(';'));
  if (opts.via?.length) q.set('via', opts.via.map((p) => `${p.lat.toFixed(6)},${p.lon.toFixed(6)}`).join(';'));
  q.set('seed', String(opts.seed));
  return q;
}

/** Sport d'une requête (liens partagés, historique, session) : absent ou inconnu = trail. */
export const requestSport = (req: Record<string, string> | undefined): Sport => (req?.sport === 'bike' ? 'bike' : 'trail');

/** Type de voie d'une requête (liens partagés, historique) : `surface`, sinon l'ancien `roads` traduit comme le fait l'API
 *  et la migration des réglages (seul `all` devient `any`) ; null si absent. */
export function requestSurface(req: Record<string, string>): Surface | null {
  if (isSurface(req.surface)) return req.surface;
  if (req.roads === 'unpaved' || req.roads === 'pedestrian' || req.roads === 'minor') return 'trail';
  if (req.roads === 'all') return 'any';
  return null;
}

/** Part (0–1) de chemin d'une sortie en « Chemins » (trail), VTT ou gravel, si elle est sous 50 % (avertissement) ; sinon null.
 *  Jamais en « Le plus court » (la préférence n'y agit pas) ni en « Route » (D53 : tout est revêtu). */
export function lowSurface(c: { trail_frac?: number }, req: Record<string, string>): { share: number } | null {
  const sf = requestSurface(req);
  if (c.trail_frac == null || !(sf === 'trail' || sf === 'mtb' || sf === 'gravel') || req.goal === 'min_distance') return null;
  return c.trail_frac < 0.5 ? { share: c.trail_frac } : null;
}

/** Réglages préremplis depuis la requête d'une boucle partagée (« Recalculer depuis ici »). */
export function settingsFromRequest(req: Record<string, string>, base: Settings): Settings {
  const sport = requestSport(req);
  const s = mergeSettings(base, sport);
  const t = CATALOG.find((c) => c.goal === (req.goal ?? 'max_dplus'))!; // goal absent = max_dplus (défaut de l'API)
  s.typeId = t.id;
  for (const f of fieldsOf(t, sport, s.surface)) if (req[f.param] != null && Number.isFinite(+req[f.param])) s.values[t.id][f.param] = +req[f.param];
  if (req.climbs === 'short' || req.climbs === 'balanced' || req.climbs === 'long') s.climbs = req.climbs;
  const rs = requestSurface(req);
  if (rs && SURFACES[sport].includes(rs)) s.surface = rs;
  s.maxGrade = req.max_grade_pct == null ? gradeDefault(sport, s.surface) : okGrade(sport, s.surface, +req.max_grade_pct) ? +req.max_grade_pct : s.maxGrade;
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

/** Durée (min). Trail : (km + D+/100) × allure. Vélo : L/v + D+/v_z selon la voie (VTT 15 km/h et 600 m/h, gravel 20 et 800,
 *  route 27 et 1 200) : estimation à recaler sur des traces réelles (synthèse sports § 1). */
export function durationMin(km: number, dplus: number, s: Pick<Settings, 'paceS' | 'surface'>, sport: Sport = 'trail'): number {
  const p = profile(sport, s.surface);
  return sport === 'bike' && p.speed && p.climb ? (km / p.speed + dplus / p.climb) * 60 : ((km + dplus / 100) * s.paceS) / 60;
}

/** Estimation du temps de calcul (s), recalée sur la Lambda réelle (scripts/lambda_bench.sh, 2026-10-06 ;
 *  api.md § Durée estimée) : n ≤ 2 au-delà de 40 km ; « longues » ×1,8 sauf en cible, où la préférence de
 *  montées n'agit plus sur la recherche ; « le plus court » ×0,6 ; +0,5 s de réseau. */
export const computeEstimateS = (D: number, goal: string, n: number, climbs: string = 'balanced') => {
  const k = D > 40 ? Math.min(n, 2) : n;
  const f = (goal === 'min_distance' ? 0.6 : 1) * (climbs === 'long' && goal !== 'target' ? 1.8 : 1);
  return Math.min(15, (3.2 + 0.085 * D) * (1 + 0.12 * (k - 1)) * f) + 0.5;
};
