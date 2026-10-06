import { describe, it, expect } from 'vitest';
import { platform, how, shouldHint, HINT_AFTER } from '../../src/lib/install.svelte';

const UA = {
  iphone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  criOS: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0 Mobile/15E148 Safari/604.1',
  mac: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15',
  pixel: 'Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36',
  win: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36',
};

describe('installation de l’app', () => {
  it('plateforme : iOS (tous navigateurs, iPad « Mac » tactile), Android, bureau', () => {
    expect(platform(UA.iphone)).toBe('ios');
    expect(platform(UA.criOS)).toBe('ios');
    expect(platform(UA.mac, 5)).toBe('ios');
    expect(platform(UA.mac, 0)).toBe('desktop');
    expect(platform(UA.pixel, 5)).toBe('android');
    expect(platform(UA.win)).toBe('desktop');
  });
  it('déjà installée : rien, même si le navigateur propose l’invite', () => {
    for (const p of ['ios', 'android', 'desktop'] as const) expect(how(p, true, true)).toBe('none');
  });
  it('invite du navigateur si disponible, sinon consigne iOS / générique Android, rien sur bureau', () => {
    expect(how('android', false, true)).toBe('prompt');
    expect(how('desktop', false, true)).toBe('prompt');
    expect(how('ios', false, false)).toBe('ios');
    expect(how('android', false, false)).toBe('manual');
    expect(how('desktop', false, false)).toBe('none');
  });
  it('suggestion : une seule fois (refus mémorisé), jamais sur bureau ni sans geste sûr', () => {
    expect(shouldHint('prompt', 'android', false)).toBe(true);
    expect(shouldHint('ios', 'ios', false)).toBe(true);
    expect(shouldHint('prompt', 'android', true)).toBe(false);
    expect(shouldHint('ios', 'ios', true)).toBe(false);
    expect(shouldHint('prompt', 'desktop', false)).toBe(false);
    expect(shouldHint('manual', 'android', false)).toBe(false);
    expect(shouldHint('none', 'ios', false)).toBe(false);
  });
  it('suggestion après le 3e itinéraire généré', () => { expect(HINT_AFTER).toBe(3); });
});
