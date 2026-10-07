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
 * sortie concernée ; jamais le départ, la zone, les points de passage, le tracé ni un identifiant.
 * `rank` : 1–4, null si inconnu (historique, lien partagé) ; temps de calcul seulement s'il est connu.
 */
export function eventBody(
  event: EventName, req: Record<string, string>, c: Pick<Candidate, 'length_m' | 'dplus_m' | 'trail_frac'> | null,
  rank: number | null, computeS: number | undefined, lang: string,
): Record<string, string | number | boolean> {
  const num = (k: string) => (req[k] != null && Number.isFinite(Number(req[k])) ? Number(req[k]) : undefined);
  const has = 'lat' in req;
  const tf = c?.trail_frac;
  const b: Record<string, string | number | boolean | undefined> = {
    event, lang,
    goal: has ? (req.goal ?? 'max_dplus') : undefined, km: num('distance_km'), dplus_m: num('dplus_m'),
    surface: req.surface, climbs: req.climbs, max_grade_pct: has ? (num('max_grade_pct') ?? 60) : undefined,
    zone: has ? 'polygon' in req : undefined, via_n: has ? (req.via ? req.via.split(';').length : 0) : undefined,
    no_repeat: req.no_repeat_junction != null ? req.no_repeat_junction !== 'false' : undefined,
    rank: rank ?? undefined,
    got_km: c ? Math.round(c.length_m / 10) / 100 : undefined, got_dplus_m: c ? Math.round(c.dplus_m) : undefined,
    trail_pct: tf != null ? Math.round(tf * 100) : undefined, road_pct: tf != null ? 100 - Math.round(tf * 100) : undefined,
    compute_s: computeS && computeS > 0 ? Math.round(computeS * 10) / 10 : undefined,
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
