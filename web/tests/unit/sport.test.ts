import { describe, it, expect, beforeEach, vi } from 'vitest';
import plan from '../fixtures/plan1.json';
import { fieldsOf, byId, profile, PROFILES, SURFACES, type Surface } from '../../src/lib/catalog';
import { buildQuery, defaultSettings, mergeSettings, setSurface, settingsFromRequest, durationMin, requestSport, lowSurface, gradeChoices } from '../../src/lib/settings';
import { sport, useSport } from '../../src/lib/sport.svelte';
import { bikeDicts, dicts, errorText, warningText, D } from '../../src/i18n/format';
import { eventBody } from '../../src/lib/hit';
import { toGpx } from '../../src/lib/gpx';
import type { Candidate } from '../../src/lib/types';

const start = { lat: 45.18, lon: 5.72 };
const c = plan.candidates[0] as unknown as Candidate;
const bike = (surface: Surface, over: object = {}) => ({ ...defaultSettings('bike'), surface, ...over });

describe('profils par sport (api.md § Sports, synthèse § 2)', () => {
  it('distance : défaut et bornes par type de voie vélo, trail inchangé', () => {
    const d = (sp: 'trail' | 'bike', sf: Surface) => { const f = fieldsOf(byId('max_dplus'), sp, sf)[0]; return [f.def, f.min, f.max]; };
    expect(d('bike', 'mtb')).toEqual([30, 5, 100]);
    expect(d('bike', 'gravel')).toEqual([50, 10, 200]);
    expect(d('bike', 'road')).toEqual([60, 10, 200]);
    for (const sf of SURFACES.trail) expect(d('trail', sf)).toEqual([10, 2, 100]);
    expect(fieldsOf(byId('target'), 'bike', 'road')[0].presets).toEqual([50, 80, 100, 120, 150, 200]);
    // D+ inchangé (le moteur ne change pas ses bornes par sport)
    expect(fieldsOf(byId('target'), 'bike', 'road')[1]).toEqual(byId('target').fields[1]);
  });
  it('« Le plus court » : distance max dans les bornes du profil (sinon 400 distance_out_of_range), trail inchangé', () => {
    const md = (sp: 'trail' | 'bike', sf: Surface) => fieldsOf(byId('shortest'), sp, sf)[1];
    const r = md('bike', 'road');
    expect([r.min, r.max, r.auto!({ dplus_m: 50 })]).toEqual([10, 200, 10]); // auto 3 km relevé à 10
    expect([md('bike', 'mtb').min, md('bike', 'mtb').max]).toEqual([5, 100]);
    const t = md('trail', 'trail'), base = byId('shortest').fields[1];
    expect([t.min, t.max, t.presets, t.auto!({ dplus_m: 500 })]).toEqual([base.min, base.max, base.presets, base.auto!({ dplus_m: 500 })]);
  });
  it('pente : défauts 60 / 25 / 22 / 0 (D78, D81 : route sans limite), vélo 5–40, « sans limite » en route seulement, le défaut est proposé', () => {
    expect([profile('trail', 'trail').grade, profile('bike', 'mtb').grade, profile('bike', 'gravel').grade, profile('bike', 'road').grade]).toEqual([60, 25, 22, 0]);
    for (const sf of SURFACES.bike) {
      const g = gradeChoices('bike', sf);
      expect([Math.min(...g.filter(Boolean)), Math.max(...g)]).toEqual([5, 40]);
      expect(g).toContain(PROFILES.bike[sf]!.grade);
      expect(g.includes(0)).toBe(sf === 'road');
    }
  });
});

describe('requête vélo', () => {
  const q = (s: ReturnType<typeof bike>, sp: 'trail' | 'bike' = 'bike') => buildQuery(s, start, { n: 1, seed: 0, sport: sp });
  it('sport=bike seulement en vélo ; surface mtb|gravel|road', () => {
    expect(buildQuery(defaultSettings(), start, { n: 1, seed: 0 }).has('sport')).toBe(false);
    expect(q(defaultSettings(), 'trail').has('sport')).toBe(false);
    expect(q(bike('gravel')).get('sport')).toBe('bike');
    expect(['mtb', 'gravel', 'road'].map((sf) => q(bike(sf as Surface)).get('surface'))).toEqual(['mtb', 'gravel', 'road']);
  });
  it('max_grade_pct absent au défaut du profil, envoyé sinon', () => {
    expect(q(bike('mtb')).has('max_grade_pct')).toBe(false); // 25
    expect(q(bike('gravel', { maxGrade: 22 })).has('max_grade_pct')).toBe(false);
    expect(q(bike('mtb', { maxGrade: 20 })).get('max_grade_pct')).toBe('20');
    expect(q(bike('road', { maxGrade: 0 })).has('max_grade_pct')).toBe(false); // route : sans limite par défaut (D78)
    expect(q(bike('road', { maxGrade: 25 })).get('max_grade_pct')).toBe('25');
  });
});

describe('réglages par sport', () => {
  it('défauts vélo : VTT, 30 km, pente 25 ; une clé par sport, la clé trail ne change pas', () => {
    const s = defaultSettings('bike');
    expect([s.surface, s.maxGrade, s.values.target.distance_km, s.values.max_dplus.distance_km]).toEqual(['mtb', 25, 30, 30]);
    expect(defaultSettings().surface).toBe('trail');
  });
  it('réglages d’un autre sport ou corrompus → défauts du sport demandé', () => {
    expect(mergeSettings({ surface: 'any', maxGrade: 0 }, 'bike')).toMatchObject({ surface: 'mtb', maxGrade: 25 });
    expect(mergeSettings({ surface: 'gravel', maxGrade: 25 }, 'bike')).toMatchObject({ surface: 'gravel', maxGrade: 25 });
    expect(mergeSettings({ surface: 'mtb', maxGrade: 22 }, 'trail')).toMatchObject({ surface: 'trail', maxGrade: 60 });
    expect(mergeSettings({ surface: 'gravel', maxGrade: 0 }, 'bike').maxGrade).toBe(22); // « sans limite » : route seulement
    expect(mergeSettings({ surface: 'road', maxGrade: 0 }, 'bike').maxGrade).toBe(0);
  });
  it('changer de type de voie : le défaut suit, une valeur choisie est ramenée dans les bornes', () => {
    const s = defaultSettings('bike');
    setSurface(s, 'bike', 'gravel');
    expect([s.surface, s.values.target.distance_km, s.maxGrade]).toEqual(['gravel', 50, 22]);
    setSurface(s, 'bike', 'road');
    expect([s.values.target.distance_km, s.maxGrade]).toEqual([60, 0]); // D78 : route sans limite
    s.values.target.distance_km = 180; // choisi
    s.maxGrade = 30;
    setSurface(s, 'bike', 'mtb');
    expect([s.values.target.distance_km, s.maxGrade]).toEqual([100, 30]);
    setSurface(s, 'bike', 'road'); // 100 km choisi, reste
    expect(s.values.target.distance_km).toBe(100);
  });
  it('trail : changer de voie ne touche à rien d’autre', () => {
    const s = defaultSettings();
    setSurface(s, 'trail', 'any');
    expect([s.surface, s.values.target.distance_km, s.maxGrade]).toEqual(['any', 10, 60]);
  });
});

describe('lien partagé et historique : le sport', () => {
  const link = { sport: 'bike', goal: 'target', distance_km: '80', dplus_m: '900', surface: 'gravel', max_grade_pct: '25', climbs: 'long' };
  it('requestSport : absent ou inconnu = trail', () => {
    expect([requestSport({}), requestSport(undefined), requestSport({ sport: 'bike' }), requestSport({ sport: 'ski' })]).toEqual(['trail', 'trail', 'bike', 'trail']);
  });
  it('lien vélo → réglages vélo préremplis', () => {
    const s = settingsFromRequest(link, defaultSettings('bike'));
    expect(s).toMatchObject({ typeId: 'target', surface: 'gravel', maxGrade: 25, climbs: 'long' });
    expect(s.values.target).toEqual({ distance_km: 80, dplus_m: 900 });
    // pente absente = défaut du profil (gravel 20), voie du trail refusée en vélo
    expect(settingsFromRequest({ sport: 'bike', goal: 'target', surface: 'gravel' }, defaultSettings('bike')).maxGrade).toBe(22);
    expect(settingsFromRequest({ sport: 'bike', goal: 'target', surface: 'any' }, defaultSettings('bike')).surface).toBe('mtb');
  });
  it('lien sans sport = trail, comme avant', () => {
    expect(settingsFromRequest({ goal: 'target', surface: 'trail' }, defaultSettings())).toMatchObject({ surface: 'trail', maxGrade: 60 });
  });
  it('avertissement « peu de chemins » : trail, VTT et gravel ; jamais en route', () => {
    expect(lowSurface({ trail_frac: 0.3 }, { sport: 'bike', surface: 'mtb' })).toEqual({ share: 0.3 });
    expect(lowSurface({ trail_frac: 0.3 }, { sport: 'bike', surface: 'gravel' })).toEqual({ share: 0.3 });
    expect(lowSurface({ trail_frac: 0.1 }, { sport: 'bike', surface: 'road' })).toBeNull();
  });
});

describe('durée estimée vélo : t = L/v + D+/v_z', () => {
  const m = (km: number, d: number, sf: Surface) => durationMin(km, d, { paceS: 360, surface: sf }, 'bike');
  it('VTT 15 km/h et 600 m/h, gravel 20 et 800, route 27 et 1 200', () => {
    expect(m(30, 800, 'mtb')).toBeCloseTo(200);
    expect(m(50, 500, 'gravel')).toBeCloseTo(187.5);
    expect(m(100, 1500, 'road')).toBeCloseTo((100 / 27 + 1.25) * 60);
  });
  it('indépendante de l’allure trail ; trail inchangé', () => {
    expect(durationMin(30, 800, { paceS: 240, surface: 'mtb' }, 'bike')).toBeCloseTo(200);
    expect(durationMin(10, 300, { paceS: 360, surface: 'trail' }, 'trail')).toBe(78);
  });
});

describe('textes par sport', () => {
  beforeEach(() => useSport('trail'));
  it('le sous-objet `bike` ne contient que des clés existantes, du même genre', () => {
    const walk = (over: unknown, base: unknown, path: string) => {
      for (const [k, v] of Object.entries(over as object)) {
        const b = (base as Record<string, unknown>)[k];
        expect(b, `${path}.${k}`).toBeDefined();
        expect(typeof v, `${path}.${k}`).toBe(typeof b);
        if (v && typeof v === 'object') walk(v, b, `${path}.${k}`);
      }
    };
    for (const l of ['fr', 'en'] as const) { const { bike: o, ...base } = dicts[l]; walk(o, base, l); }
  });
  it('vélo : FR sortie vélo, EN « ride », erreurs vers Gravel ; trail : Mixte / Mixed', () => {
    expect(D('fr').app.tagline).toBe('Ta sortie trail sur mesure');
    expect(D('fr').surface.any).toBe('Mixte');
    expect(D('en').surface.any).toBe('Mixed');
    expect(D('en').compute.find).toBe('Find my run');
    expect(errorText('fr', 'paved_network_too_short')).toContain('« Mixte »');
    useSport('bike');
    expect(sport.id).toBe('bike');
    expect(D('fr').app.tagline).toBe('Ta sortie vélo sur mesure');
    expect(D('fr').compute.find).toBe('Trouver ma sortie');
    expect(D('en').compute.find).toBe('Find my ride');
    expect(D('en').result.otherLoopsN({ n: '2' })).toBe('Other rides (2)');
    expect(D('en').history.title).toBe('My routes');
    expect(errorText('fr', 'paved_network_too_short')).toContain('« Gravel »');
    expect(errorText('en', 'paved_network_major_roads')).toContain('“Gravel”');
    expect(D('fr').surface.help.road).toContain('pistes cyclables');
    expect(D('en').surface.parts.mixed).toBe('paved path');
    expect(bikeDicts.fr.more.maxGrade).toBe('Pente max en montée (sur 50 m)');
    expect(D('fr').more.descent({ g: '22' })).toBe('Descente : jusqu’à 22 %');
  });
  it('major_roads_share (E2) : un texte dans chaque langue et chaque sport', () => {
    expect(warningText('fr', 'major_roads_share')).toBe('Une part importante de la sortie emprunte de grandes routes.');
    expect(warningText('en', 'major_roads_share')).toBe('A large share of this run follows major roads.');
    useSport('bike');
    expect(warningText('en', 'major_roads_share')).toBe('A large share of this ride follows major roads.');
  });
});

describe('événements, GPX', () => {
  const req = { lat: '45.18', lon: '5.72', goal: 'target', distance_km: '80', dplus_m: '900', surface: 'gravel' };
  it('`sport` dans les événements : bike, sinon trail ; absent sans requête', () => {
    expect(eventBody('gpx', { ...req, sport: 'bike' }, null, 1, 3, 'fr').sport).toBe('bike');
    expect(eventBody('gpx', req, null, 1, 3, 'fr').sport).toBe('trail');
    expect('sport' in eventBody('shared_open', {}, null, null, 0, 'fr')).toBe(false);
  });
  it('pente max des événements : défaut du profil quand la requête ne la porte pas', () => {
    expect(eventBody('gpx', { ...req, sport: 'bike', surface: 'mtb' }, null, 1, 3, 'fr').max_grade_pct).toBe(25);
    expect(eventBody('gpx', { ...req, sport: 'bike', surface: 'road' }, null, 1, 3, 'fr').max_grade_pct).toBe(0);
    expect(eventBody('gpx', { ...req, surface: 'trail' }, null, 1, 3, 'fr').max_grade_pct).toBe(60);
  });
  it('GPX cycling en vélo, trail_running sinon', () => {
    expect(toGpx(c, 'd', 'bike')).toContain('<type>cycling</type>');
    expect(toGpx(c, 'd')).toContain('<type>trail_running</type>');
  });
});
