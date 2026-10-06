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

/** D33 : « N montées · la plus longue G m sur ℓ km » (vide si le serveur n'a pas renvoyé `climbs`). */
export function climbsText(c: { climbs?: { count: number; longest_gain_m: number; longest_len_m: number } }, lang: Lang): string {
  const k = c.climbs, d = dicts[lang].detail;
  if (!k) return '';
  if (!k.count) return d.noClimb;
  return d.climbsLine({ n: num(lang, k.count), plural: k.count > 1 ? 's' : '', gain: num(lang, k.longest_gain_m), len: km(lang, k.longest_len_m) });
}
