// Agrégations de la page /admin (réponse de GET /api/admin/stats, contracts/admin.md § 4).
// Une vue `null` (requête Insights non terminée) ou un champ absent s'affichent « — ».

type Day = { day: string };
export type Stats = {
  from: string; to: string; incomplete: boolean; scanned_mb: number;
  visitors_by_day: (Day & { visitors: number; hits: number })[] | null;
  calcs_by_day: (Day & { n: number; ok: number; rejected: number })[] | null;
  codes: { code: string; n: number; rate: number }[] | null;
  rejected_codes: { code: string; n: number }[] | null;
  compute_s: { p50: number | null; p95: number | null; n: number; by_day: (Day & { p50: number; p95: number; n: number })[] | null } | null;
  /** D70 : calculs par sport (absent des anciennes réponses) */
  sports?: { sport: string; n: number }[] | null;
  goals: { goal: string; n: number }[] | null;
  /** v2 (D77) : voie qualifiée par le sport (absent = trail pour les anciennes réponses) */
  surfaces: { sport?: string; surface: string; n: number }[] | null;
  climbs: { climbs: string; n: number }[] | null;
  hist_km: Hist | null;
  hist_dplus_m: Hist | null;
  starts: Cell[] | null;
  starts_outside: Cell[] | null;
  countries: { country: string; visitors: number; hits: number }[] | null;
  regions: { country: string; region: string; visitors: number; hits: number }[] | null;
  devices: { dev: string; visitors: number }[] | null;
  referrers: { ref: string; hits: number }[] | null;
  /** D59 : actions par jour (clés = événements), taux d'action, combinaisons */
  events_by_day?: (Day & Partial<Record<EventName, number>>)[] | null;
  action_rates?: Record<RateDim, Rate[]> | null;
  filter?: Filter | null;
  /** liens créés / ouverts vus par le serveur (POST 201 / GET 200 de /api/loops), tous sports, sans refus de mesure */
  shares?: { created: number; opened: number } | null;
  modes?: Mode[] | null;
  top_combos?: { sport?: string; goal: string; surface: string; km_bin: string; dplus_bin: string; rank: string; share: number; gpx: number }[] | null;
};
/** Filtres appliqués par le serveur, et surveillance par mode (v2). */
export type Filter = { sport?: string; surface?: string; goal?: string };
export type Mode = { sport: string; surface: string; goal: string; n: number; fail: number; timeout: number; p50: number | null; p95: number | null; cost_usd: number | null };
export type EventName = 'share_click' | 'share_created' | 'gpx' | 'shared_open';
export type RateDim = 'sport' | 'goal' | 'surface' | 'km' | 'dplus_m' | 'rank';
export type Rate = { key: string; calcs: number; share: number; gpx: number; share_rate: number | null; gpx_rate: number | null };

export const EVENTS: [EventName, string][] = [['share_click', 'Clics partager'], ['share_created', 'Liens créés'], ['gpx', 'GPX exportés'], ['shared_open', 'Liens ouverts']];
export const DIMS: [RateDim, string][] = [['sport', 'Sport'], ['goal', 'Type de sortie'], ['surface', 'Sport et voie'], ['km', 'Distance'], ['dplus_m', 'D+'], ['rank', 'Rang de la sortie']];

/** Libellé d'une clé de taux ou de combinaison : tranches de 5 km / 250 m, rang. */
export function keyLabel(dim: RateDim | 'km_bin' | 'dplus_bin', k: string): string {
  if (k === '-' || k === '') return DASH;
  const x = Number(k);
  if (dim === 'km' || dim === 'km_bin') return `${fmt(x)}–${fmt(x + 5)} km`;
  if (dim === 'dplus_m' || dim === 'dplus_bin') return `${fmt(x)}–${fmt(x + 250)} m`;
  if (dim === 'rank') return `n° ${k}`;
  if (dim === 'sport') return sportLabel(k);
  if (dim === 'surface') return k.includes('/') ? surfaceLabel(...(k.split('/') as [string, string])) : surfaceLabel('trail', k);
  return label(k);
}

/** Total d'un événement sur la période (`null` : vue absente ou aucune action enregistrée). */
export function eventTotal(s: Stats, e: EventName): number | null {
  const rows = s.events_by_day;
  return rows?.length ? sum(rows.map((r) => r[e] ?? 0)) : null;
}
export type Hist = { step: number; bins: { from: number; n: number }[] };
export type Cell = { lat: number; lon: number; n: number };

export const DASH = '—';

/** Nombre en français, « — » si absent. */
export function fmt(x: number | null | undefined, digits = 0): string {
  return x == null || !Number.isFinite(x) ? DASH : x.toLocaleString('fr-FR', { maximumFractionDigits: digits, minimumFractionDigits: digits });
}

export function pct(x: number | null | undefined): string {
  return x == null || !Number.isFinite(x) ? DASH : `${fmt(x * 100, 1)} %`;
}

const iso = (d: Date) => d.toISOString().slice(0, 10);

/** Jours AAAA-MM-JJ de `from` à `to` inclus (UTC). */
export function days(from: string, to: string): string[] {
  const out: string[] = [];
  for (let d = new Date(`${from}T00:00:00Z`); iso(d) <= to && out.length < 500; d.setUTCDate(d.getUTCDate() + 1)) out.push(iso(d));
  return out;
}

/** Période prédéfinie (jours) se terminant aujourd'hui (UTC). */
export function preset(n: number, today = new Date()): { from: string; to: string } {
  const from = new Date(today);
  from.setUTCDate(from.getUTCDate() - (n - 1));
  return { from: iso(from), to: iso(today) };
}

/** Valeurs par jour (`fill` si le jour manque : 0 pour un comptage, NaN pour un temps) ; `null` si la vue manque. */
export function perDay<T extends Day>(ds: string[], rows: T[] | null | undefined, f: (r: T) => number, fill = 0): number[] | null {
  if (!rows) return null;
  const m = new Map(rows.map((r) => [r.day, f(r)]));
  return ds.map((d) => m.get(d) ?? fill);
}

const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

/** Tuiles : visiteurs (visiteurs-jours), visites, calculs, taux d'échec, temps p50/p95. */
export function tiles(s: Stats) {
  const v = s.visitors_by_day, c = s.calcs_by_day;
  const n = c ? sum(c.map((r) => r.n)) : null;
  const ok = c ? sum(c.map((r) => r.ok)) : null;
  // aucune ligne `hit` (mesure pas encore en service) : « — » plutôt que 0
  return {
    visitors: v?.length ? sum(v.map((r) => r.visitors)) : null,
    visits: v?.length ? sum(v.map((r) => r.hits)) : null,
    calcs: n,
    rejected: c ? sum(c.map((r) => r.rejected)) : null,
    failRate: n && ok != null ? (n - ok) / n : null,
    p50: s.compute_s?.p50 ?? null,
    p95: s.compute_s?.p95 ?? null,
  };
}

/** Histogramme complété (classes vides entre la première et la dernière). */
export function histBins(h: Hist | null): { labels: string[]; values: number[] } | null {
  if (!h) return null;
  if (!h.bins.length) return { labels: [], values: [] };
  const m = new Map(h.bins.map((b) => [b.from, b.n]));
  const lo = Math.min(...m.keys()), hi = Math.max(...m.keys());
  const labels: string[] = [], values: number[] = [];
  for (let x = lo; x <= hi && labels.length < 200; x += h.step) {
    labels.push(`${fmt(x)}–${fmt(x + h.step)}`);
    values.push(m.get(x) ?? 0);
  }
  return { labels, values };
}

/** Libellés visibles (« - » = champ absent, lignes antérieures au journal v2). */
const LABELS: Record<string, string> = {
  max_dplus: 'Max D+', target: 'Cible', min_distance: 'Le plus court',
  trail: 'Chemins', any: 'Mixte', road: 'Route', mtb: 'VTT', gravel: 'Gravel',
  short: 'Courtes', balanced: 'Équilibrées', long: 'Longues',
  mobile: 'Mobile', desktop: 'Ordinateur', '-': DASH,
};
export const label = (k: string) => LABELS[k] ?? k;
/** Sport (D70) : « trail » est aussi un type de voie (« Chemins ») ; ici c'est le sport. Absent = trail. */
export const sportLabel = (k: string) => (k === 'bike' ? 'Vélo' : k === 'trail' || k === '-' ? 'Trail' : k);

/** Voie qualifiée par le sport : « Vélo · Route » ; voie absente (`-`) : « Trail · — ». */
export const surfaceLabel = (sport: string, surface: string) => `${sportLabel(sport)} · ${label(surface)}`;

/** Options du menu « Sport et voie » : [valeur, libellé] ; la valeur est `sport` ou `sport/voie`. */
export const SPORT_OPTIONS: [string, string][] = [
  ['', 'Tous'], ['trail', 'Trail'], ['trail/trail', 'Trail · Chemins'], ['trail/any', 'Trail · Mixte'], ['trail/road', 'Trail · Route'],
  ['bike', 'Vélo'], ['bike/mtb', 'Vélo · VTT'], ['bike/gravel', 'Vélo · Gravel'], ['bike/road', 'Vélo · Route'],
];
export const GOAL_OPTIONS: [string, string][] = [['', 'Tous'], ['max_dplus', 'Max D+'], ['target', 'Cible'], ['min_distance', 'Le plus court']];

/** Filtre de l'API depuis les valeurs des deux menus (champs vides omis). */
export function filterOf(sportSurface: string, goal: string): Filter {
  const [sport, surface] = sportSurface.split('/');
  return { ...(sport && { sport }), ...(surface && { surface }), ...(goal && { goal }) };
}

/** Query string de GET /api/admin/stats. */
export const statsQuery = (range: { from: string; to: string }, f: Filter) => new URLSearchParams({ from: range.from, to: range.to, ...f }).toString();

/** Rappel du filtre actif : « Vélo · Route, Max D+ » ('' sans filtre). */
export function filterText(f: Filter | null | undefined): string {
  if (!f) return '';
  const where = f.sport ? (f.surface ? surfaceLabel(f.sport, f.surface) : sportLabel(f.sport)) : '';
  return [where, f.goal && label(f.goal)].filter(Boolean).join(', ');
}

/** Vue de la page selon `location.hash` (défaut : Visu). */
export const viewOf = (hash: string): 'visu' | 'surveillance' => (hash === '#surveillance' ? 'surveillance' : 'visu');

/** Totaux de la vue `modes` (`null` si absente) : calculs, échecs, timeouts et leur taux. */
export function modeTotals(modes: Mode[] | null | undefined) {
  if (!modes) return null;
  const n = sum(modes.map((m) => m.n)), timeout = sum(modes.map((m) => m.timeout));
  return { n, fail: sum(modes.map((m) => m.fail)), timeout, timeoutRate: n ? timeout / n : null };
}

/** Coût Lambda d'un calcul : 5 décimales (quelques dixièmes de millième de dollar). */
export const costCalc = (x: number | null | undefined) => (x == null ? DASH : `${fmt(x, 5)} $`);

/** Parts d'un regroupement : [{ label, n, share }] trié par effectif. */
export function shares<T>(rows: T[] | null, key: (r: T) => string, n: (r: T) => number, sport?: (r: T) => string) {
  if (!rows) return null;
  const total = sum(rows.map(n));
  return rows.map((r) => ({ sport: sport?.(r), label: label(key(r)), n: n(r), share: total ? n(r) / total : 0 })).sort((a, b) => b.n - a.n);
}

/** Consommation AWS du mois (GET /api/admin/usage) : postes, seuils gratuits, projection, budget. */
export type UsageLine = {
  key: string; service: string; item: string; unit: string | null; free?: number;
  /** jauge (stockage) : taille actuelle */
  now?: number | null;
  used: number | null; projected: number | null; share: number | null; projected_share?: number | null;
  cost: number | null; cost_forecast: number | null;
};
export type Usage = {
  month: string; renewal: string; elapsed: number; fetched_at: number;
  plan: { type: string; status: string; credits_usd: number | null } | null;
  spend: { actual: number; forecast: number; forecast_source: 'budgets' | 'rythme'; limit: number | null; steps: number[]; next_step: number | null } | null;
  lines: UsageLine[]; errors: string[];
};

/** Quantité d'un poste ; en Mo sous 1 Go. */
export function qty(x: number | null | undefined, unit: string | null): string {
  if (x == null || unit == null) return DASH;
  if (unit.startsWith('Go') && unit !== 'Go·s' && x < 1) return `${fmt(x * 1024, 1)} ${unit.replace('Go', 'Mo')}`;
  return `${fmt(x, x < 10 && !Number.isInteger(x) ? 1 : 0)} ${unit}`;
}

/** Montant en dollars : 3 décimales sous 1 $. */
export function usd(x: number | null | undefined): string {
  if (x == null || !Number.isFinite(x)) return DASH;
  if (x === 0) return '0 $';
  if (x < 0.001) return '< 0,001 $';
  return `${fmt(x, x < 1 ? 3 : 2)} $`;
}
