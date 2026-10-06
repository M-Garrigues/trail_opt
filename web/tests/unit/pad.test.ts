import { describe, it, expect } from 'vitest';
import { CATALOG, bump, padKey, padParse } from '../../src/lib/catalog';

const km = CATALOG[0].fields[0], dplus = CATALOG[1].fields[1], maxKm = CATALOG[2].fields[1];
const type = (keys: string[], decimals: boolean, raw = '') => keys.reduce((r, k) => padKey(r, k, decimals), raw);

describe('pavé numérique (mobile)', () => {
  it('saisie : chiffres, zéro de tête, 5 chiffres au plus', () => {
    expect(type(['1', '2'], true)).toBe('12');
    expect(type(['0', '7'], true)).toBe('7');
    expect(type(['1', '2', '3', '4', '5', '6'], false)).toBe('12345');
  });
  it('virgule : km seulement, une seule, une décimale', () => {
    expect(type(['1', '2', ',', '5', '7'], true)).toBe('12,5');
    expect(type([',', '5'], true)).toBe('0,5');
    expect(type(['1', ',', ',', '5'], true)).toBe('1,5');
    expect(type(['1', ','], false)).toBe('1');
  });
  it('touche 00 (mètres)', () => {
    expect(type(['5', '00'], false)).toBe('500');
    expect(type(['00'], false)).toBe('');
    expect(type(['1', '2', '3', '4', '00'], false)).toBe('1234');
  });
  it('effacement', () => {
    expect(type(['back'], true, '12,5')).toBe('12,');
    expect(type(['back', 'back', 'back', 'back', 'back'], true, '12,5')).toBe('');
  });
  it('bornes, arrondi au pas, vide', () => {
    expect(padParse('12,5', km)).toEqual({ ok: true, v: 12.5 });
    expect(padParse('12,3', km)).toEqual({ ok: true, v: 12.5 });
    expect(padParse('1', km)).toEqual({ ok: false, v: 2 });
    expect(padParse('250', km)).toEqual({ ok: false, v: 100 });
    expect(padParse('', km)).toEqual({ ok: false, v: null });
    expect(padParse('', maxKm)).toEqual({ ok: true, v: null });
    expect(padParse('304', dplus)).toEqual({ ok: true, v: 300 });
  });
  it('−/+ : 1 km ou 50 m, aligné, borné', () => {
    expect(bump(km, 10, 1)).toBe(11);
    expect(bump(km, 12.5, 1)).toBe(13);
    expect(bump(km, 12.5, -1)).toBe(12);
    expect(bump(km, 2, -1)).toBe(2);
    expect(bump(km, 100, 1)).toBe(100);
    expect(bump(dplus, 300, 1)).toBe(350);
    expect(bump(dplus, 10, -1)).toBe(10);
  });
  it('les puces sont dans les bornes', () => {
    for (const t of CATALOG) for (const f of t.fields) for (const p of f.presets) { expect(p).toBeGreaterThanOrEqual(f.min); expect(p).toBeLessThanOrEqual(f.max); }
  });
});
