// Mesure d'audience sans cookie (contracts/admin.md § 2) : un POST /api/hit à l'ouverture du site.
// Rien n'est lu ni écrit sur l'appareil hormis la case Réglages (`optrail.nostats`).
import { load } from './store';
import { sha256hex, sharedId } from './share';

export type HitEnv = { dev: boolean; host: string; path: string; gpc: unknown; dnt: string | null; off: boolean };

/** Jamais en développement, en local, sur /admin, avec GPC / Do Not Track, ni case décochée. */
export function shouldSend(e: HitEnv): boolean {
  return !e.dev && e.host !== 'localhost' && e.host !== '127.0.0.1' && e.path !== '/admin' && e.gpc !== true && e.dnt !== '1' && !e.off;
}

/** Corps : page (accueil ou sortie partagée), site d'origine réduit à son nom d'hôte (pas le nôtre), langue. */
export function hitBody(path: string, referrer: string, host: string, lang: string): { page: 'home' | 'shared'; ref?: string; lang: string } {
  let ref: string | undefined;
  try {
    ref = new URL(referrer).hostname || undefined;
  } catch { /* pas de référent */ }
  return { page: sharedId(path) ? 'shared' : 'home', ...(ref && ref !== host ? { ref } : {}), lang };
}

export async function sendHit(lang: string): Promise<void> {
  const env: HitEnv = {
    dev: import.meta.env.DEV, host: location.hostname, path: location.pathname,
    gpc: (navigator as { globalPrivacyControl?: unknown }).globalPrivacyControl, dnt: navigator.doNotTrack, off: load('nostats', false),
  };
  if (!shouldSend(env)) return;
  const body = new TextEncoder().encode(JSON.stringify(hitBody(location.pathname, document.referrer, location.hostname, lang)));
  try {
    await fetch('/api/hit', { method: 'POST', body, keepalive: true, headers: { 'content-type': 'application/json', 'x-amz-content-sha256': await sha256hex(body) } });
  } catch { /* mesure d'audience : jamais bloquante */ }
}
