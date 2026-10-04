import { describe, it, expect, beforeEach, vi } from 'vitest';
import plan from '../fixtures/plan1.json';
import * as hist from '../../src/lib/history';
import { defaultSettings } from '../../src/lib/settings';
import { roundStart, simplify, inPolygon, parseLatLon } from '../../src/lib/geo';
import type { Candidate } from '../../src/lib/types';

class Mem {
  m = new Map<string, string>();
  quota = Infinity;
  getItem(k: string) { return this.m.get(k) ?? null; }
  setItem(k: string, v: string) {
    if (v.length > this.quota) throw new DOMException('full', 'QuotaExceededError');
    this.m.set(k, v);
  }
  removeItem(k: string) { this.m.delete(k); }
  key(i: number) { return [...this.m.keys()][i] ?? null; }
  get length() { return this.m.size; }
}
let mem: Mem;
beforeEach(() => { mem = new Mem(); vi.stubGlobal('localStorage', mem); });

const c = plan.candidates[0] as unknown as Candidate;
const entry = () => ({ settings: defaultSettings(), start: { lat: 48.73, lon: 2.27 }, candidate: c, warnings: [], request: { lat: '48.73', lon: '2.27' }, effective_start: plan.effective_start as hist.Entry['effective_start'], lower_bound_m: null, data_version: 'd', solver_version: 's' });

describe('historique (D24)', () => {
  it('51 boucles → 50, plus récente d’abord', () => {
    let last = '';
    for (let i = 0; i < 51; i++) last = hist.add(entry()).entry.id;
    const l = hist.list();
    expect(l.length).toBe(50);
    expect(l[0].id).toBe(last);
  });
  it('suppression unitaire et effacement', () => {
    const a = hist.add(entry()).entry, b = hist.add(entry()).entry;
    hist.removeEntry(a.id);
    expect(hist.list().map((e) => e.id)).toEqual([b.id]);
    hist.clear();
    expect(hist.list()).toEqual([]);
  });
  it('géométrie compacte ≈ 10 Ko', () => {
    const e = hist.add(entry()).entry;
    expect(JSON.stringify(e).length).toBeLessThan(12000);
    expect(e.candidate.ele.every(Number.isInteger)).toBe(true);
    expect(e.candidate.sig).toBeUndefined(); // géométrie réduite : signature invalide, partage masqué
  });
  it('QuotaExceededError → anciennes retirées, sans plantage', () => {
    for (let i = 0; i < 5; i++) hist.add(entry());
    const one = JSON.stringify(hist.list().slice(0, 1)).length;
    mem.quota = one * 3; // place pour ~3 boucles
    const r = hist.add(entry());
    expect(r.status).toBe('ok');
    expect(hist.list()[0].id).toBe(r.entry.id);
    expect(hist.list().length).toBeLessThanOrEqual(3);
    mem.quota = 10;
    expect(hist.add(entry()).status).toBe('full');
  });
  it('localStorage indisponible → liste vide, pas d’exception', () => {
    vi.stubGlobal('localStorage', { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); } });
    expect(hist.list()).toEqual([]);
    expect(hist.add(entry()).status).toBe('unavailable');
  });
});

describe('géo', () => {
  it('dernière position à 0,01°', () => {
    expect(roundStart({ lat: 48.73094, lon: 2.27136 })).toEqual({ lat: 48.73, lon: 2.27 });
  });
  it('Douglas–Peucker garde les extrémités', () => {
    const k = simplify(c.lat, c.lon, 10);
    expect(k[0]).toBe(0);
    expect(k[k.length - 1]).toBe(c.lat.length - 1);
    expect(k.length).toBeLessThan(c.lat.length);
  });
  it('point dans polygone, saisie lat, lon', () => {
    const sq: [number, number][] = [[0, 0], [1, 0], [1, 1], [0, 1]];
    expect(inPolygon([0.5, 0.5], sq)).toBe(true);
    expect(inPolygon([1.5, 0.5], sq)).toBe(false);
    expect(parseLatLon('48,7309; 2,2713')).toEqual({ lat: 48.7309, lon: 2.2713 });
    expect(parseLatLon('48.73, 2.27')).toEqual({ lat: 48.73, lon: 2.27 });
    expect(parseLatLon('Massy')).toBeNull();
  });
});
