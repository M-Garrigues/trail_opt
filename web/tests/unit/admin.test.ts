import { describe, it, expect } from 'vitest';
import { days, eventTotal, fmt, histBins, keyLabel, label, pct, perDay, preset, shares, tiles, type Stats } from '../../src/admin/stats';
import { eventBody, hitBody, shouldSend, type HitEnv } from '../../src/lib/hit';
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

describe('page admin : actions (D59)', () => {
  it('totaux, libellés des tranches et rangs', () => {
    expect([eventTotal(s, 'share_created'), eventTotal(s, 'gpx'), eventTotal(s, 'shared_open')]).toEqual([2, 2, 1]);
    expect(eventTotal({ ...s, events_by_day: [] }, 'gpx')).toBeNull();
    expect(eventTotal({ ...s, events_by_day: undefined }, 'gpx')).toBeNull();
    expect(keyLabel('km', '10')).toBe('10–15 km');
    expect(keyLabel('dplus_bin', '250')).toBe('250–500 m');
    expect(keyLabel('rank', '2')).toBe('n° 2');
    expect(keyLabel('surface', '-')).toBe('—');
    expect(keyLabel('goal', 'target')).toBe('Cible');
    expect(s.action_rates!.rank[0].share_rate).toBeCloseTo(1 / 3, 2);
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
  it('forcé en test e2e seulement pour le local, jamais contre GPC ou la case', () => {
    const local = { ...env, dev: true, host: 'localhost', forced: true };
    expect(shouldSend(local)).toBe(true);
    expect(shouldSend({ ...local, gpc: true })).toBe(false);
    expect(shouldSend({ ...local, off: true })).toBe(false);
  });
  it('événement : réglages et stats, sans départ ni identifiant', () => {
    const req = { lat: '45.18', lon: '5.72', goal: 'target', distance_km: '10', dplus_m: '300', climbs: 'balanced', surface: 'trail',
      no_repeat_junction: 'true', n_candidates: '3', seed: '0', polygon: '5.7,45.1;5.8,45.1;5.8,45.2', via: '45.1,5.7;45.2,5.8' };
    const b = eventBody('gpx', req, { length_m: 10_234, dplus_m: 312.4, trail_frac: 0.853 }, 2, 4.26, 'fr');
    expect(b).toEqual({ event: 'gpx', lang: 'fr', goal: 'target', km: 10, dplus_m: 250, surface: 'trail', climbs: 'balanced', max_grade_pct: 60,
      zone: true, via_n: 2, no_repeat: true, rank: 2, got_km: 10, got_dplus_m: 250, trail_pct: 85, road_pct: 15, compute_s: 4 });
    // trois classes (aménagé) et « Sorties plus fluides » forcée
    const b3 = eventBody('gpx', { ...req, smooth: 'false' }, { length_m: 10_234, dplus_m: 312.4, trail_frac: 0.7, surface_share: [0.6, 0.2, 0.2] }, 2, 4.26, 'fr');
    expect([b3.trail_pct, b3.mixed_pct, b3.road_pct, b3.smooth]).toEqual([60, 20, 20, false]);
    expect(JSON.stringify(b)).not.toMatch(/45\.18|5\.72|seed|polygon/);
    // lien partagé / historique : rang et temps inconnus ; requête vide : pas de réglages
    expect(eventBody('shared_open', {}, { length_m: 5000, dplus_m: 100 }, null, 0, 'en')).toEqual({ event: 'shared_open', lang: 'en', got_km: 5, got_dplus_m: 0 });
  });
  it('page, site d’origine, langue', () => {
    expect(hitBody('/', 'https://www.google.com/search?q=x', 'optrail.eu', 'fr')).toEqual({ page: 'home', ref: 'www.google.com', lang: 'fr' });
    expect(hitBody('/b/AbCdEf123', '', 'optrail.eu', 'en')).toEqual({ page: 'shared', lang: 'en' });
    expect(hitBody('/', 'https://optrail.eu/b/x', 'optrail.eu', 'fr')).toEqual({ page: 'home', lang: 'fr' });
  });
});
