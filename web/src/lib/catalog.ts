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
const distance: Field = { param: 'distance_km', unit: 'km', def: 10, min: 2, max: 100, step: 0.5 };

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
    fields: [distance, { param: 'dplus_m', unit: 'm', def: 300, min: 10, max: 5000, step: 10 }],
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
      { param: 'dplus_m', unit: 'm', def: 500, min: 50, max: 10000, step: 10 },
      {
        param: 'max_distance_km', unit: 'km', def: null, min: 2, max: 100, step: 0.5, optional: true,
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
