import { describe, it, expect } from 'vitest';
import { idxAt, parseVia } from '../../src/lib/geo';

describe('repères (api.md v1.5)', () => {
  it('l’abscisse dist_m donne l’index du tracé (pas de NaN)', () => {
    const c = { dist: [0, 100, 250, 400, 1000] };
    expect(idxAt(c, 240)).toBe(2);
    expect(idxAt(c, 5230)).toBe(4);
    expect(Number.isFinite(idxAt(c, 0))).toBe(true);
  });
  it('via = lat,lon', () => {
    expect(parseVia('45.2,5.7;45.3,5.8')[0]).toEqual({ lat: 45.2, lon: 5.7 });
  });
});

import { nearestMark } from '../../src/lib/geo';

describe('symboles des repères du profil (retour du fondateur, 2026-10-07)', () => {
  it('le pointeur prend le symbole le plus proche, dans la tolérance', () => {
    expect(nearestMark([100, 108, 200], 106)).toBe(1); // symboles qui se chevauchent : le plus proche
    expect(nearestMark([100, 108, 200], 103)).toBe(0);
    expect(nearestMark([100, 200], 150)).toBe(-1); // entre deux, trop loin des deux
    expect(nearestMark([100], 118, 20)).toBe(0); // toucher : tolérance plus large
    expect(nearestMark([100], 118, 12)).toBe(-1); // souris
    expect(nearestMark([], 10)).toBe(-1);
  });
});

import { dplusBetween } from '../../src/lib/geo';

describe('D+ des étapes (D67 : hystérésis de 3 m, comme le moteur)', () => {
  it('compte une montée de 3 m, ignore les oscillations de moins de 3 m', () => {
    expect(dplusBetween([100, 103, 101, 104], 0, 3)).toBe(4);
    expect(dplusBetween([100, 102, 100, 102, 100], 0, 4)).toBe(0);
    // sens pas encore établi : un creux de 1 m au départ ne sert pas de référence (comme `updown_hyst`)
    expect(dplusBetween([100, 99, 101.5], 0, 2)).toBe(0);
  });
});
