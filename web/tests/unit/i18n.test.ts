import { describe, it, expect } from 'vitest';
import codes from '../../../engine/codes.json';
import { fr } from '../../src/i18n/fr';
import { en } from '../../src/i18n/en';
import { errorText, warningText, duration, formatParams, REJECTED } from '../../src/i18n/format';

type C = { code: string; kind: 'error' | 'warning'; params: string[] };
const all = codes as C[];

describe('couverture des codes D14 (engine/codes.json)', () => {
  for (const c of all) {
    it(`${c.code} a un texte fr et en`, () => {
      const p = Object.fromEntries(c.params.map((k) => [k, 12.5]));
      for (const lang of ['fr', 'en'] as const) {
        if (c.kind === 'error') {
          const dict = lang === 'fr' ? fr : en;
          const handled = c.code in dict.err || REJECTED.has(c.code) || ['no_loop_found', 'invariant_violated', 'invalid_problem'].includes(c.code);
          expect(handled, c.code).toBe(true);
          if (c.code !== 'cancelled') expect(errorText(lang, c.code, p)).not.toBe('');
        } else {
          expect(c.code in (lang === 'fr' ? fr : en).warn, c.code).toBe(true);
          if (!['profile_mismatch', 'long_distance'].includes(c.code)) expect(warningText(lang, c.code, p)).not.toBe('');
        }
      }
    });
  }
  it('les paramètres déclarés apparaissent dans le texte', () => {
    expect(errorText('fr', 'distance_out_of_range', { min_km: 2, max_km: 100 })).toBe('Distance entre 2 et 100 km.');
    expect(errorText('en', 'zone_too_large', { area_km2: 2100.44, max_km2: 1965 })).toBe('Area too large (2,100.4 km², max 1,965).');
    expect(warningText('fr', 'access_round_trip', { access_m: 54.0 })).toBe('Inclut un aller-retour d’accès de 54 m.');
  });
  it('code inconnu → générique avec le code ; validation → « refusée »', () => {
    expect(errorText('fr', 'zzz_new')).toBe('Erreur interne (zzz_new). Réessaie.');
    expect(errorText('en', 'http_502')).toBe('Internal error (http_502). Try again.');
    expect(errorText('fr', 'roads_unknown')).toBe('Requête refusée (roads_unknown). Recharge la page.');
  });
  it('dplus_unreachable_proven sans min_km', () => {
    expect(errorText('fr', 'dplus_unreachable_proven', { dplus_m: 800, min_km: null })).toBe('Pas assez de relief ici.');
    expect(errorText('fr', 'dplus_unreachable_proven', { dplus_m: 800, min_km: 12.34 })).toBe('+800 m impossible : il faut au moins 12,3 km.');
  });
});

describe('formats', () => {
  it('durée ~1 h 05', () => {
    expect(duration(65)).toBe('~1 h 05');
    expect(duration(45.2)).toBe('~45 min');
    expect(duration(120)).toBe('~2 h 00');
  });
  it('Intl selon la langue', () => {
    expect(formatParams('fr', { km: 10.49, dplus_m: 1234.4 })).toEqual({ km: '10,5', dplus_m: '1\u202f234' });
    expect(formatParams('en', { km: 10.49, dplus_m: 1234.4 })).toEqual({ km: '10.5', dplus_m: '1,234' });
  });
});
