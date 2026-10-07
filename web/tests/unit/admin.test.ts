import { describe, it, expect } from 'vitest';
import { days, fmt, histBins, label, pct, perDay, preset, shares, tiles, type Stats } from '../../src/admin/stats';
import { hitBody, shouldSend, type HitEnv } from '../../src/lib/hit';
import fixture from '../fixtures/admin-stats.json';

const s = fixture as unknown as Stats;

describe('page admin : agrégations', () => {
  it('tuiles (visiteurs-jours, taux d’échec, temps)', () => {
    expect(tiles(s)).toEqual({ visitors: 15, visits: 40, calcs: 20, rejected: 9, failRate: 0.1, p50: 4.1, p95: 9.6 });
    // vues absentes : « — »
    const t = tiles({ ...s, visitors_by_day: null, calcs_by_day: null, compute_s: null });
    expect([t.visitors, t.calcs, t.failRate, t.p50]).toEqual([null, null, null, null]);
    expect(fmt(t.visitors)).toBe('—');
    expect(pct(t.failRate)).toBe('—');
    expect(pct(0.1)).toBe('10,0 %');
    // aucune ligne `hit` encore (journaux actuels) : « — », pas 0
    expect(tiles({ ...s, visitors_by_day: [] }).visitors).toBeNull();
  });
  it('jours de la période, valeurs par jour complétées de zéros', () => {
    expect(days('2026-09-29', '2026-10-02')).toEqual(['2026-09-29', '2026-09-30', '2026-10-01', '2026-10-02']);
    expect(preset(7, new Date('2026-10-07T12:00:00Z'))).toEqual({ from: '2026-10-01', to: '2026-10-07' });
    const ds = days(s.from, s.to);
    expect(perDay(ds, s.visitors_by_day, (r) => r.visitors)).toEqual([0, 0, 0, 0, 0, 12, 3]);
    expect(perDay(ds, null, () => 1)).toBeNull();
    expect(perDay(ds, s.compute_s!.by_day, (r) => r.p50, NaN)!.slice(4)).toEqual([NaN, 4.0, 4.3]);
  });
  it('histogrammes avec classes vides, parts par catégorie', () => {
    expect(histBins({ step: 5, bins: [{ from: 20, n: 1 }, { from: 10, n: 4 }] })).toEqual({ labels: ['10–15', '15–20', '20–25'], values: [4, 0, 1] });
    expect(histBins(null)).toBeNull();
    const g = shares(s.surfaces, (r) => r.surface, (r) => r.n)!;
    expect(g[0]).toEqual({ label: '—', n: 6, share: 2 / 3 });
    expect(label('max_dplus')).toBe('Max D+');
  });
});

describe('mesure d’audience : envoi', () => {
  const env: HitEnv = { dev: false, host: 'optrail.eu', path: '/', gpc: undefined, dnt: null, off: false };
  it('conditions', () => {
    expect(shouldSend(env)).toBe(true);
    for (const e of [{ dev: true }, { host: 'localhost' }, { path: '/admin' }, { gpc: true }, { dnt: '1' }, { off: true }]) {
      expect(shouldSend({ ...env, ...e }), JSON.stringify(e)).toBe(false);
    }
  });
  it('page, site d’origine, langue', () => {
    expect(hitBody('/', 'https://www.google.com/search?q=x', 'optrail.eu', 'fr')).toEqual({ page: 'home', ref: 'www.google.com', lang: 'fr' });
    expect(hitBody('/b/AbCdEf123', '', 'optrail.eu', 'en')).toEqual({ page: 'shared', lang: 'en' });
    expect(hitBody('/', 'https://optrail.eu/b/x', 'optrail.eu', 'fr')).toEqual({ page: 'home', lang: 'fr' });
  });
});
