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
