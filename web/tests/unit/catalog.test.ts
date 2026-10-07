import { describe, it, expect } from 'vitest';
import bounds from '../fixtures/api-bounds.json';
import plan from '../fixtures/plan1.json';
import { CATALOG, byId, clampField } from '../../src/lib/catalog';
import { buildQuery, defaultSettings, mergeSettings, durationMin, settingsFromRequest, MAX_GRADES, computeEstimateS, lowSurface } from '../../src/lib/settings';
import { surfaceText, lowSurfaceText, labelsText } from '../../src/i18n/format';
import type { PlanResponse } from '../../src/lib/types';

describe('catalogue (ui-spec §2)', () => {
  it('bornes identiques à api.md', () => {
    for (const t of CATALOG) {
      expect(bounds.goals).toContain(t.goal);
      for (const f of t.fields) {
        const b = f.param === 'dplus_m' ? (bounds.dplus_m as Record<string, number[]>)[t.goal] : (bounds as unknown as Record<string, number[]>)[f.param];
        expect([f.min, f.max], `${t.id}.${f.param}`).toEqual(b);
      }
    }
  });
  it('ramène hors bornes (1 km → 2)', () => {
    const d = byId('max_dplus').fields[0];
    expect(clampField(d, 1)).toEqual({ v: 2, changed: true });
    expect(clampField(d, 10.3)).toEqual({ v: 10.5, changed: true });
    expect(clampField(d, 12)).toEqual({ v: 12, changed: false });
  });
  it('distance max auto = clamp(D+/25, 3, 60)', () => {
    const f = byId('shortest').fields[1];
    expect(f.auto!({ dplus_m: 500 })).toBe(20);
    expect(f.auto!({ dplus_m: 50 })).toBe(3);
    expect(f.auto!({ dplus_m: 5000 })).toBe(60);
  });
  it('chiffres vedettes', () => {
    const r = plan as unknown as PlanResponse, c = r.candidates[0];
    expect(byId('max_dplus').headline(c, r, 'fr')).toBe('+424 m');
    expect(byId('shortest').headline(c, r, 'en')).toBe('10.5 km');
    expect(byId('target').headline({ ...c, target_gap: { distance_m: 200, dplus_m: 10 } }, r, 'fr')).toBe('10,5 km · +424 m (+10)');
  });
});

describe('requête', () => {
  it('seulement des paramètres de api.md, défauts ui-spec', () => {
    const q = buildQuery(defaultSettings(), { lat: 48.7309, lon: 2.2713 }, { n: 1, seed: 0 });
    for (const k of q.keys()) expect(bounds.params).toContain(k);
    expect(q.get('goal')).toBe('target'); // Cible : type par défaut
    expect(q.get('distance_km')).toBe('10');
    expect(q.get('dplus_m')).toBe('300');
    expect(q.get('surface')).toBe('trail');
    expect(q.has('roads')).toBe(false);
    expect(q.get('n_candidates')).toBe('1');
    expect(q.has('max_grade_pct')).toBe(false);
  });
  it('le plus court : dplus_m, pas de distance_km, max auto absent', () => {
    const s = defaultSettings();
    s.typeId = 'shortest';
    const q = buildQuery(s, { lat: 45, lon: 5.8 }, { n: 4, seed: 3, polygon: [[5.8, 45], [5.9, 45], [5.9, 45.1]] });
    expect(q.get('goal')).toBe('min_distance');
    expect(q.get('dplus_m')).toBe('500');
    expect(q.has('distance_km')).toBe(false);
    expect(q.has('max_distance_km')).toBe(false);
    expect(q.get('polygon')).toBe('5.80000,45.00000;5.90000,45.00000;5.90000,45.10000');
    expect(q.get('seed')).toBe('3');
  });
  it('type de voie (api.md v1.7) : ancien `roads` mémorisé ou partagé, traduit', () => {
    expect(mergeSettings({ roads: 'minor' }).surface).toBe('trail'); // ancien défaut → nouveau défaut
    expect(mergeSettings({ roads: 'unpaved' }).surface).toBe('trail');
    expect(mergeSettings({ roads: 'all' }).surface).toBe('any');
    expect(mergeSettings({ surface: 'road', roads: 'all' }).surface).toBe('road');
    expect('roads' in mergeSettings({ roads: 'all' })).toBe(false);
    const s = defaultSettings();
    expect(settingsFromRequest({ goal: 'target', roads: 'unpaved' }, s).surface).toBe('trail');
    expect(settingsFromRequest({ goal: 'target', roads: 'minor' }, s).surface).toBe('trail'); // comme le réglage mémorisé et l'API
    expect(settingsFromRequest({ goal: 'target', roads: 'all' }, s).surface).toBe('any');
    expect(settingsFromRequest({ goal: 'target', surface: 'road', roads: 'minor' }, s).surface).toBe('road');
    expect(settingsFromRequest({ lat: '45', lon: '5' }, s).typeId).toBe('max_dplus'); // goal absent = défaut de l'API
  });
  it('part de chemin et avertissement sous 50 % du type voulu', () => {
    expect(surfaceText({ trail_frac: 0.724 }, 'fr')).toBe('72 % chemin · 28 % route');
    expect(surfaceText({}, 'fr')).toBe('');
    // trois classes (v1.8) : arrondi à 100 %, parts nulles omises ; `trail_frac` des anciennes sorties sinon
    expect(surfaceText({ trail_frac: 0.6, surface_share: [0.555, 0.105, 0.34] }, 'fr')).toBe('56 % chemin · 10 % aménagé · 34 % route');
    expect(surfaceText({ surface_share: [0.333, 0.333, 0.334] }, 'en')).toBe('33% trail · 33% mixed · 34% road');
    expect(surfaceText({ surface_share: [0, 0.004, 0.996] }, 'fr')).toBe('100 % route');
    expect(lowSurface({ trail_frac: 0.38 }, { surface: 'trail' })).toEqual({ share: 0.38 });
    expect(lowSurfaceText(lowSurface({ trail_frac: 0.38 }, { surface: 'trail' }), 'fr')).toBe('Seulement 38 % de chemins ici : peu de sentiers autour de ce départ.');
    expect(lowSurface({ trail_frac: 0.1 }, { surface: 'road' })).toBeNull(); // D53 : « Route » = revêtu par construction
    expect(lowSurface({ trail_frac: 0.38 }, { surface: 'any' })).toBeNull();
    expect(lowSurface({ trail_frac: 0.38 }, { roads: 'all' })).toBeNull();
    expect(lowSurface({ trail_frac: 0.38 }, { roads: 'minor' })).toEqual({ share: 0.38 });
    // « Le plus court » : la préférence n'agit pas, pas d'avertissement
    expect(lowSurface({ trail_frac: 0.3 }, { goal: 'min_distance', surface: 'trail' })).toBeNull();
    expect(lowSurface({ trail_frac: 0.3 }, { goal: 'target', surface: 'trail' })).toEqual({ share: 0.3 });
    expect(lowSurface({ trail_frac: 0.6 }, { surface: 'trail' })).toBeNull();
    expect(lowSurface({}, { surface: 'trail' })).toBeNull();
  });
  it('étiquettes (v1.8) : calme, balisé, eau ; mention OSM seulement avec une part OSM', () => {
    expect(labelsText({ calm_frac: 0.823, hike_m: 3400, water_m: 1200 }, 'fr')).toEqual({ text: '82 % au calme · 3,4 km balisés · 1,2 km au bord de l’eau', osm: true });
    expect(labelsText({ calm_frac: 0.5, hike_m: 0, water_m: 20 }, 'en')).toEqual({ text: '50% quiet', osm: false });
    expect(labelsText({}, 'fr')).toEqual({ text: '', osm: false });
  });
  it('« Sorties plus fluides » (D62) : rien d’envoyé par défaut, forcé sinon, relu depuis un lien, mémorisé', () => {
    const s = defaultSettings();
    expect(s.smooth).toBe('auto');
    const q = (x: typeof s) => buildQuery(x, { lat: 45, lon: 5 }, { n: 1, seed: 0 });
    expect(q(s).has('smooth')).toBe(false);
    expect(q({ ...s, smooth: 'on' }).get('smooth')).toBe('true');
    expect(q({ ...s, smooth: 'off' }).get('smooth')).toBe('false');
    expect(settingsFromRequest({ goal: 'target', smooth: 'true' }, s).smooth).toBe('on');
    expect(settingsFromRequest({ goal: 'target', smooth: '0' }, s).smooth).toBe('off');
    expect(settingsFromRequest({ goal: 'target' }, { ...s, smooth: 'on' }).smooth).toBe('auto');
    expect(mergeSettings({ smooth: 'off' }).smooth).toBe('off');
    expect(mergeSettings({ smooth: 'maybe' }).smooth).toBe('auto');
  });
  it('réglages mémorisés corrompus → défauts', () => {
    expect(mergeSettings('x').typeId).toBe('target');
    expect(mergeSettings({ typeId: 'max_dplus' }).typeId).toBe('max_dplus'); // le dernier type choisi reste prioritaire
    expect(mergeSettings({ typeId: 'nope', values: { target: { dplus_m: 700 } } }).values.target).toEqual({ distance_km: 10, dplus_m: 700 });
  });
  it('pente max (api.md v1.2) : défaut 60 % absent, « sans limite » = 0, bornes 5–60', () => {
    const s = defaultSettings();
    expect(s.maxGrade).toBe(bounds.max_grade_pct_default);
    const g = MAX_GRADES.filter(Boolean);
    expect([Math.min(...g), Math.max(...g)]).toEqual(bounds.max_grade_pct);
    expect(MAX_GRADES).toContain(0);
    s.maxGrade = 0;
    expect(buildQuery(s, { lat: 45, lon: 5.8 }, { n: 1, seed: 0 }).get('max_grade_pct')).toBe('0');
    s.maxGrade = 25;
    expect(buildQuery(s, { lat: 45, lon: 5.8 }, { n: 1, seed: 0 }).get('max_grade_pct')).toBe('25');
    // ancien réglage « désactivé » → 60 ; valeur hors liste → 60
    expect(mergeSettings({ maxGradeOn: false, maxGrade: 30 }).maxGrade).toBe(60);
    expect(mergeSettings({ maxGradeOn: true, maxGrade: 30 }).maxGrade).toBe(30);
    expect(mergeSettings({ maxGrade: 70 }).maxGrade).toBe(60);
    expect(mergeSettings({ maxGrade: 0 }).maxGrade).toBe(0);
    // lien partagé : absent → 60, 0 → sans limite
    expect(settingsFromRequest({ goal: 'max_dplus' }, s).maxGrade).toBe(60);
    expect(settingsFromRequest({ goal: 'max_dplus', max_grade_pct: '0' }, s).maxGrade).toBe(0);
  });
  it('durée = (km + D+/100) × allure', () => {
    expect(durationMin(10, 300, 360)).toBe(78);
  });
});

describe('durée de calcul estimée (api.md v1.6)', () => {
  it('n ramené à 2 au-delà de 40 km', () => {
    expect(computeEstimateS(10, 'max_dplus', 1)).toBeCloseTo(4.05 + 0.5); // Lambda réelle : 4,0 s
    expect(computeEstimateS(50, 'max_dplus', 4)).toBeCloseTo(computeEstimateS(50, 'max_dplus', 2)); // n ≤ 2 au-delà de 40 km
    expect(computeEstimateS(100, 'max_dplus', 2)).toBeCloseTo(11.7 * 1.12 + 0.5); // réelle : 13,7 s
    expect(computeEstimateS(10, 'max_dplus', 3, 'long')).toBeCloseTo(4.05 * 1.24 * 1.8 + 0.5); // réelle : 8,8 s
    expect(computeEstimateS(10, 'target', 3, 'long')).toBeCloseTo(computeEstimateS(10, 'target', 3)); // pas en cible
    expect(computeEstimateS(10, 'min_distance', 3)).toBeCloseTo(4.05 * 1.24 * 0.6 + 0.5);
    expect(computeEstimateS(100, 'max_dplus', 2, 'long')).toBeCloseTo(15.5); // plafond
  });
});
