// Historique local (D24) : chaque boucle reçue, FIFO à 50, géométrie simplifiée ~10 m.
import { load, saveOrThrow } from './store';
import { simplify } from './geo';
import type { Candidate, Msg, PlanResponse } from './types';
import type { Settings, Start } from './settings';

export const MAX_ENTRIES = 50;
const KEY = 'history';

export type Entry = {
  id: string;
  ts: number;
  settings: Settings;
  start: Start;
  candidate: Candidate;
  warnings: Msg[];
  /** paramètres envoyés à /api/plan (pour le partage) */
  request: Record<string, string>;
  effective_start: PlanResponse['effective_start'];
  lower_bound_m: number | null;
  data_version: string;
  solver_version: string;
};

export function list(): Entry[] {
  const h = load<Entry[]>(KEY, []);
  return Array.isArray(h) ? h : [];
}

/** Géométrie réduite : DP 10 m, 5 décimales, altitude entière (≈ 10 Ko par boucle). */
export function compact(c: Candidate): Candidate {
  const keep = simplify(c.lat, c.lon, 10);
  const r5 = (v: number) => Math.round(v * 1e5) / 1e5;
  const { sig: _sig, ...rest } = c; // géométrie réduite : la signature ne vaut plus
  return {
    ...rest,
    lat: keep.map((i) => r5(c.lat[i])),
    lon: keep.map((i) => r5(c.lon[i])),
    ele: keep.map((i) => Math.round(c.ele[i])),
    dist: keep.map((i) => Math.round(c.dist[i])),
  };
}

/** Écrit la liste ; si le quota est atteint, retire la plus ancienne et réessaie.
 *  Renvoie 'ok', 'full' (l'entrée nouvelle n'a pas pu être gardée) ou 'unavailable'. */
export function write(entries: Entry[]): 'ok' | 'full' | 'unavailable' {
  let h = entries.slice(0, MAX_ENTRIES);
  for (;;) {
    try {
      saveOrThrow(KEY, h);
      return 'ok';
    } catch (e) {
      const quota = e instanceof DOMException && (e.name === 'QuotaExceededError' || e.code === 22);
      if (!quota) return 'unavailable';
      if (h.length <= 1) {
        try { saveOrThrow(KEY, []); } catch { /* rien */ }
        return 'full';
      }
      h = h.slice(0, -1);
    }
  }
}

export function add(e: Omit<Entry, 'id' | 'ts'>): { entry: Entry; status: ReturnType<typeof write> } {
  const entry: Entry = { ...e, candidate: compact(e.candidate), id: Math.random().toString(36).slice(2, 10), ts: Date.now() };
  return { entry, status: write([entry, ...list()]) };
}

export function removeEntry(id: string) {
  return write(list().filter((e) => e.id !== id));
}

export function clear() {
  return write([]);
}
