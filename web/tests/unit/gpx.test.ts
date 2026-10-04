import { describe, it, expect } from 'vitest';
import plan from '../fixtures/plan1.json';
import { toGpx, gpxFilename, loopName } from '../../src/lib/gpx';
import type { Candidate } from '../../src/lib/types';

const c = plan.candidates[0] as unknown as Candidate;

describe('GPX', () => {
  it('trk, ele, attribution, tous les points', () => {
    const g = toGpx(c, plan.data_version);
    expect(g).toMatch(/^<\?xml/);
    expect(g).toContain('<trk>');
    expect(g).toContain('© IGN');
    expect(g).toContain('Etalab 2.0');
    expect((g.match(/<trkpt /g) ?? []).length).toBe(c.lat.length);
    expect((g.match(/<ele>/g) ?? []).length).toBe(c.lat.length);
    expect(g).toContain(`<trkpt lat="${c.lat[0].toFixed(6)}" lon="${c.lon[0].toFixed(6)}">`);
  });
  it('nom et fichier neutres en langue', () => {
    expect(loopName(c)).toBe('optrail 10.5 km +424 m');
    expect(gpxFilename(c)).toBe('optrail-10.5km-424m.gpx');
  });
});
