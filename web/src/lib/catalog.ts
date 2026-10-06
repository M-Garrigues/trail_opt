// Catalogue des types d'entraînement (ui-spec §2, D22). Ajouter un type = une entrée + ses clés i18n + un goal API.
import type { Candidate, PlanResponse } from './types';
import { num, type Lang } from '../i18n/format';

export type Param = 'distance_km' | 'dplus_m' | 'max_distance_km';
export type Values = Partial<Record<Param, number | null>>;
export type Field = {
  param: Param;
  unit: 'km' | 'm';
  def: number | null;
  min: number;
  max: number;
  step: number;
  /** valeurs courantes proposées en puces dans le pavé (mobile) */
  presets: number[];
  optional?: boolean;
  auto?: (v: Values) => number;
};
export type TypeId = 'max_dplus' | 'target' | 'shortest';
export type TrainingType = {
  id: TypeId;
  goal: 'max_dplus' | 'target' | 'min_distance';
  icon: string; // chemin SVG (viewBox 24)
  fields: Field[];
  headline: (c: Candidate, r: PlanResponse, lang: Lang) => string;
  enabled: boolean;
};

const clamp = (x: number, a: number, b: number) => Math.min(b, Math.max(a, x));
const signed = (lang: Lang, v: number) => (v >= 0 ? '+' : '−') + num(lang, Math.abs(v));
const KM = [5, 10, 15, 21, 30, 42];
const distance: Field = { param: 'distance_km', unit: 'km', def: 10, min: 2, max: 100, step: 0.5, presets: KM };

export const CATALOG: TrainingType[] = [
  {
    id: 'max_dplus',
    goal: 'max_dplus',
    icon: 'M2 20 L7 11 L10 15 L15 6 L18 11 L22 4 M22 4 L22 9 M22 4 L17 4',
    fields: [distance],
    headline: (c, _r, lang) => `+${num(lang, c.dplus_m)} m`,
    enabled: true,
  },
  {
    id: 'target',
    goal: 'target',
    icon: 'M12 3 a9 9 0 1 0 0.01 0 M12 7 a5 5 0 1 0 0.01 0 M12 11 a1 1 0 1 0 0.01 0',
    fields: [distance, { param: 'dplus_m', unit: 'm', def: 300, min: 10, max: 5000, step: 10, presets: [300, 500, 1000, 1500, 2000] }],
    headline: (c, _r, lang) =>
      `${num(lang, c.length_m / 1000, 1)} km · +${num(lang, c.dplus_m)} m` +
      (c.target_gap ? ` (${signed(lang, c.target_gap.dplus_m)})` : ''),
    enabled: true,
  },
  {
    id: 'shortest',
    goal: 'min_distance',
    icon: 'M3 20 L21 8 M3 20 L21 20 M15 4 a3 3 0 1 0 0.01 0 M15 2 L15 4',
    fields: [
      { param: 'dplus_m', unit: 'm', def: 500, min: 50, max: 10000, step: 10, presets: [300, 500, 1000, 1500, 2000] },
      {
        param: 'max_distance_km', unit: 'km', def: null, min: 2, max: 100, step: 0.5, presets: KM, optional: true,
        auto: (v) => clamp((v.dplus_m ?? 500) / 25, 3, 60),
      },
    ],
    headline: (c, _r, lang) => `${num(lang, c.length_m / 1000, 1)} km`,
    enabled: true,
  },
];

export const byId = (id: string) => CATALOG.find((t) => t.id === id) ?? CATALOG[0];

export function defaults(t: TrainingType): Values {
  return Object.fromEntries(t.fields.map((f) => [f.param, f.def]));
}

/** Ramène dans les bornes et au pas ; renvoie aussi si la valeur a été modifiée. */
export function clampField(f: Field, v: number): { v: number; changed: boolean } {
  if (!Number.isFinite(v)) return { v: f.def ?? f.min, changed: true };
  const c = clamp(Math.round(v / f.step) * f.step, f.min, f.max);
  const r = Math.round(c * 100) / 100;
  return { v: r, changed: Math.abs(r - v) > 1e-9 };
}

/** Ajustement fin −/+ du pavé (mobile) : 1 km ou 50 m, la valeur s'aligne sur le pas. */
export function bump(f: Field, v: number, dir: 1 | -1): number {
  const s = f.unit === 'km' ? 1 : 50;
  const n = dir > 0 ? Math.floor(v / s + 1e-9) * s + s : Math.ceil(v / s - 1e-9) * s - s;
  return clamp(n, f.min, f.max);
}

/** Pavé numérique intégré (mobile) : texte saisi après l'appui sur une touche ('0'…'9', '00', ',', 'back').
 *  Au plus 5 chiffres avant la virgule, une décimale, virgule seulement si `decimals`. */
export function padKey(raw: string, key: string, decimals: boolean): string {
  if (key === 'back') return raw.slice(0, -1);
  const [int, dec] = raw.split(',');
  if (key === ',') return !decimals || dec != null ? raw : (raw || '0') + ',';
  if (!/^\d+$/.test(key)) return raw;
  if (dec != null) return dec.length || key.length > 1 ? raw : raw + key;
  if (int === '' || int === '0') return key === '00' ? int : key;
  return int.length + key.length > 5 ? raw : raw + key;
}

/** Valeur du texte saisi : `ok` = validable (dans les bornes, ou vide si le champ est facultatif = auto) ;
 *  `v` = valeur arrondie au pas et ramenée dans les bornes (null si vide). */
export function padParse(raw: string, f: Field): { ok: boolean; v: number | null } {
  if (raw === '') return { ok: !!f.optional, v: null };
  const x = parseFloat(raw.replace(',', '.'));
  return { ok: x >= f.min && x <= f.max, v: clampField(f, x).v };
}
