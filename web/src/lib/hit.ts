// Mesure d'audience sans cookie (contracts/admin.md § 2, D59) : un POST /api/hit à l'ouverture du site,
// et un par action suivie (partage, GPX, lien partagé ouvert). Rien n'est lu ni écrit sur l'appareil
// hormis la case Réglages (`optrail.nostats`).
import { load } from './store';
import { sha256hex, sharedId } from './share';
import type { Candidate } from './types';

/** `forced` : tests e2e seulement (`window.optrailHitTest`), lève l'exclusion du développement local. */
export type HitEnv = { dev: boolean; host: string; path: string; gpc: unknown; dnt: string | null; off: boolean; forced?: boolean };

/** Jamais en développement, en local, sur /admin, avec GPC / Do Not Track, ni case décochée. */
export function shouldSend(e: HitEnv): boolean {
  const local = e.dev || e.host === 'localhost' || e.host === '127.0.0.1';
  return (!local || !!e.forced) && e.path !== '/admin' && e.gpc !== true && e.dnt !== '1' && !e.off;
}

/** Corps : page (accueil ou sortie partagée), site d'origine réduit à son nom d'hôte (pas le nôtre), langue. */
export function hitBody(path: string, referrer: string, host: string, lang: string): { page: 'home' | 'shared'; ref?: string; lang: string } {
  let ref: string | undefined;
  try {
    ref = new URL(referrer).hostname || undefined;
  } catch { /* pas de référent */ }
  return { page: sharedId(path) ? 'shared' : 'home', ...(ref && ref !== host ? { ref } : {}), lang };
}

export type EventName = 'share_click' | 'share_created' | 'gpx' | 'shared_open';

/**
 * Action suivie (D59) : réglages de la demande (`req` = paramètres de GET /api/plan) et stats de la
 * sortie concernée, distances et D+ par tranches de 5 km / 250 m ; jamais le départ, la zone, les points de
 * passage, le tracé ni un identifiant (le serveur n'y ajoute pas le visiteur du jour).
 * `rank` : 1–4, null si inconnu (historique, lien partagé) ; temps de calcul seulement s'il est connu.
 */
export function eventBody(
  event: EventName, req: Record<string, string>, c: (Pick<Candidate, 'length_m' | 'dplus_m' | 'trail_frac'> & Pick<Partial<Candidate>, 'surface_share'>) | null,
  rank: number | null, computeS: number | undefined, lang: string,
): Record<string, string | number | boolean> {
  const num = (k: string) => (req[k] != null && Number.isFinite(Number(req[k])) ? Number(req[k]) : undefined);
  // tranches des vues admin (revue M4) : rien d'exact qui rattacherait l'action à la ligne du calcul
  const bin = (x: number | undefined, step: number) => (x == null ? undefined : Math.floor(x / step) * step);
  const has = 'lat' in req;
  // parts de voie : trois classes (chemin, aménagé, route) si la sortie les a, sinon l'ancienne part de chemin
  const sh = c?.surface_share, tf = c?.trail_frac;
  const pct = (x: number) => Math.round(x * 100);
  const b: Record<string, string | number | boolean | undefined> = {
    event, lang,
    goal: has ? (req.goal ?? 'max_dplus') : undefined, km: bin(num('distance_km'), 5), dplus_m: bin(num('dplus_m'), 250),
    surface: req.surface, climbs: req.climbs, max_grade_pct: has ? (num('max_grade_pct') ?? 60) : undefined,
    zone: has ? 'polygon' in req : undefined, via_n: has ? (req.via ? req.via.split(';').length : 0) : undefined,
    no_repeat: req.no_repeat_junction != null ? req.no_repeat_junction !== 'false' : undefined,
    smooth: req.smooth != null ? req.smooth === 'true' || req.smooth === '1' : undefined, // D62 : seulement si forcé
    rank: rank ?? undefined,
    got_km: c ? bin(c.length_m / 1000, 5) : undefined, got_dplus_m: c ? bin(c.dplus_m, 250) : undefined,
    trail_pct: sh ? pct(sh[0]) : tf != null ? pct(tf) : undefined, mixed_pct: sh ? pct(sh[1]) : undefined,
    road_pct: sh ? pct(sh[2]) : tf != null ? 100 - pct(tf) : undefined,
    compute_s: computeS && computeS > 0 ? Math.round(computeS) : undefined,
  };
  return Object.fromEntries(Object.entries(b).filter(([, v]) => v !== undefined)) as Record<string, string | number | boolean>;
}

function env(): HitEnv {
  return {
    dev: import.meta.env.DEV, host: location.hostname, path: location.pathname,
    gpc: (navigator as { globalPrivacyControl?: unknown }).globalPrivacyControl, dnt: navigator.doNotTrack, off: load('nostats', false),
    forced: (window as { optrailHitTest?: boolean }).optrailHitTest === true,
  };
}

async function post(payload: object): Promise<void> {
  if (!shouldSend(env())) return;
  const body = new TextEncoder().encode(JSON.stringify(payload));
  try {
    await fetch('/api/hit', { method: 'POST', body, keepalive: true, headers: { 'content-type': 'application/json', 'x-amz-content-sha256': await sha256hex(body) } });
  } catch { /* mesure d'audience : jamais bloquante */ }
}

export const sendHit = (lang: string) => post(hitBody(location.pathname, document.referrer, location.hostname, lang));
export const sendEvent = (body: Record<string, string | number | boolean>) => post(body);
