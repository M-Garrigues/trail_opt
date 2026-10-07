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

import { placeLabels, markPrio } from '../../src/lib/geo';

describe('noms des repères du profil sans chevauchement (2026-10-07)', () => {
  it('rangée libre la plus basse, sinon masqué, par priorité', () => {
    // a et b se chevauchent, c est à part ; b prioritaire (sommet plus haut)
    expect(placeLabels([[0, 50], [40, 90], [100, 140]], [1, 2, 0])).toEqual([1, 0, 0]);
    // trois noms au même endroit sur deux rangées : le moins prioritaire masqué
    expect(placeLabels([[0, 50], [0, 50], [0, 50]], [3, 1, 2])).toEqual([0, -1, 1]);
    // marge de 4 px entre deux noms
    expect(placeLabels([[0, 50], [52, 90]], [1, 0], 1)).toEqual([0, -1]);
    expect(placeLabels([[0, 50], [54, 90]], [1, 0], 1)).toEqual([0, 0]);
  });
  it('priorité : point de passage, puis sommet avant col, puis le plus haut', () => {
    const m = (kind: 'via' | 'col' | 'summit', ele: number | null) => markPrio({ kind, ele }, 500);
    expect(m('via', null)).toBeGreaterThan(m('summit', 3000));
    expect(m('summit', 800)).toBeGreaterThan(m('col', 2000));
    expect(m('col', 1300)).toBeGreaterThan(m('col', 1200));
    expect(m('col', null)).toBe(500); // altitude inconnue : celle du profil
  });
});
