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
