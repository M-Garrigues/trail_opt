import { fr } from './fr';
import { en, type Dict } from './en';

export type Lang = 'fr' | 'en';
// D40 : pointeur fin (bureau) → « clique » au lieu de « touche » (ou « tap » → « click »).
const fine = typeof matchMedia !== 'undefined' && matchMedia('(pointer: fine)').matches;
const clicky = (s: string) => s.replace(/\b([Tt])ouche (la|le)\b/g, (_, t, a) => `${t === 'T' ? 'C' : 'c'}lique sur ${a}`).replace(/\bTap\b/g, 'Click').replace(/\btap\b/g, 'click');
function patch<T>(o: T): T {
  if (typeof o === 'string') return clicky(o) as T;
  if (typeof o === 'function') return ((...a: unknown[]) => patch((o as (...x: unknown[]) => unknown)(...a))) as T;
  if (Array.isArray(o)) return o.map(patch) as T;
  if (o && typeof o === "object") return Object.fromEntries(Object.entries(o).map(([k, v]) => [k, patch(v)])) as T;
  return o;
}
export const dicts: Record<Lang, Dict> = fine ? { fr: patch(fr), en: patch(en) } : { fr, en };
export type Params = Record<string, number | string | null | undefined>;

export function num(lang: Lang, v: number, maxFrac = 0): string {
  return new Intl.NumberFormat(lang, { maximumFractionDigits: maxFrac }).format(v);
}
export const km = (lang: Lang, m: number, frac = 1) => `${num(lang, m / 1000, frac)} km`;
export const meters = (lang: Lang, m: number) => `${num(lang, m)} m`;

/** Durée en minutes → « ~1 h 05 » ou « ~45 min ». */
export function duration(min: number): string {
  const t = Math.max(1, Math.round(min));
  if (t < 60) return `~${t} min`;
  return `~${Math.floor(t / 60)} h ${String(t % 60).padStart(2, '0')}`;
}

/** Paramètres serveur → chaînes formatées : *_km et *_km2 à 1 décimale, le reste entier. */
export function formatParams(lang: Lang, p: Params = {}): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(p)) {
    if (v == null) out[k] = '';
    else if (typeof v === 'number') out[k] = num(lang, v, /_?km2?$/.test(k) ? 1 : 0);
    else out[k] = v;
  }
  return out;
}

/** Codes de validation : « Requête refusée ({code}) ». Le reste inconnu : « Erreur interne ». */
export const REJECTED = new Set([
  'invalid_request', 'mode_unknown', 'roads_unknown', 'target_dplus_required',
  'tolerance_out_of_range', 'max_grade_invalid', 'time_out_of_range', 'climbs_unknown',
]);

export function errorText(lang: Lang, code: string, params?: Params): string {
  const d = dicts[lang];
  const p = formatParams(lang, params);
  if (code in d.err && !['rejected', 'internal'].includes(code)) return d.err[code as keyof Dict['err']](p);
  if (REJECTED.has(code)) return d.err.rejected({ code });
  return d.err.internal({ code });
}

export function warningText(lang: Lang, code: string, params?: Params): string {
  const d = dicts[lang];
  if (!(code in d.warn)) return '';
  return d.warn[code as keyof Dict['warn']](formatParams(lang, params));
}

/** api.md v1.7 : « 72 % chemin · 28 % route » (vide si le serveur n'a pas renvoyé `trail_frac`). */
export function surfaceText(c: { trail_frac?: number; surface_share?: number[] }, lang: Lang): string {
  const s = dicts[lang].surface;
  if (c.surface_share?.length === 3) {
    // api.md v1.8 : trois parts arrondies à 100 % (plus forts restes), les parts nulles omises
    const raw = c.surface_share.map((x) => 100 * x);
    const pct = raw.map(Math.floor);
    const order = raw.map((x, i) => [x - pct[i], i]).sort((a, b) => b[0] - a[0]);
    for (let k = 0; k < 100 - pct.reduce((a, b) => a + b, 0) && k < 3; k++) pct[order[k][1]]++;
    const names = [s.parts.trail, s.parts.mixed, s.parts.road];
    const sep = lang === 'fr' ? ' % ' : '% ';
    return pct.map((p, i) => (p ? `${p}${sep}${names[i]}` : '')).filter(Boolean).join(' · ');
  }
  if (c.trail_frac == null) return '';
  const trail = Math.round(100 * c.trail_frac);
  return dicts[lang].surface.share({ trail: String(trail), road: String(100 - trail) });
}

/** Étiquettes (api.md v1.8) : « 82 % au calme · 3,4 km balisés · 1,2 km au bord de l'eau » ; `osm` si une part vient
 *  d'OpenStreetMap (mention obligatoire). Balisage et eau absents ou nuls : rien (0 = rien de connu). */
export function labelsText(c: { calm_frac?: number; hike_m?: number; water_m?: number }, lang: Lang): { text: string; osm: boolean } {
  const l = dicts[lang].labels;
  const hike = (c.hike_m ?? 0) >= 50, water = (c.water_m ?? 0) >= 50;
  const parts = [
    c.calm_frac != null ? l.calm({ pct: String(Math.round(100 * c.calm_frac)) }) : '',
    hike ? l.hike({ km: km(lang, c.hike_m!) }) : '',
    water ? l.water({ km: km(lang, c.water_m!) }) : '',
  ].filter(Boolean);
  return { text: parts.join(' · '), osm: hike || water };
}

/** Avertissement « peu de chemins / de routes » de la sortie affichée (`lowSurface`), '' sinon. */
export function lowSurfaceText(low: { share: number } | null, lang: Lang): string {
  return low ? dicts[lang].surface.lowTrail({ pct: String(Math.round(100 * low.share)) }) : '';
}

/** D33 : « N montées · la plus longue G m sur ℓ km » (vide si le serveur n'a pas renvoyé `climbs`). */
export function climbsText(c: { climbs?: { count: number; longest_gain_m: number; longest_len_m: number } }, lang: Lang): string {
  const k = c.climbs, d = dicts[lang].detail;
  if (!k) return '';
  if (!k.count) return d.noClimb;
  return d.climbsLine({ n: num(lang, k.count), plural: k.count > 1 ? 's' : '', gain: num(lang, k.longest_gain_m), len: km(lang, k.longest_len_m) });
}
