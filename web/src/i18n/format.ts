import { fr } from './fr';
import { en, type Dict } from './en';

export type Lang = 'fr' | 'en';
export const dicts: Record<Lang, Dict> = { fr, en };
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
