// Partage (api.md v1.1) : POST /api/loops → {id}, GET /api/loops/<id>. URL : /b/<id>.
import { PlanError } from './api';
import type { Candidate, Msg, PlanResponse } from './types';

export type Shared = {
  request: Record<string, string>;
  data_version: string;
  solver_version: string;
  effective_start: PlanResponse['effective_start'];
  zone: PlanResponse['zone'] | null;
  candidate: Candidate;
  warnings: Msg[];
};

async function errorOf(res: Response): Promise<PlanError> {
  if (res.status === 429) return new PlanError('service_paused', {}, 429); // coupe-circuit (D32 B2) ; busy = 503 du stockage
  try {
    const e = (await res.json())?.error;
    if (e?.code) return new PlanError(e.code, e.params ?? {}, res.status);
  } catch { /* corps vide */ }
  return new PlanError(`http_${res.status}`, {}, res.status);
}

export async function postLoop(shared: Shared, token: string | null): Promise<string> {
  const body = new TextEncoder().encode(JSON.stringify(shared));
  // CloudFront OAC exige le SHA-256 du corps pour un POST vers la Function URL
  const sha = [...new Uint8Array(await crypto.subtle.digest('SHA-256', body))].map((b) => b.toString(16).padStart(2, '0')).join('');
  let res: Response;
  try {
    res = await fetch('/api/loops', {
      method: 'POST', body,
      headers: { 'content-type': 'application/json', 'x-amz-content-sha256': sha, ...(token ? { 'x-turnstile-token': token } : {}) },
    });
  } catch {
    throw new PlanError(navigator.onLine ? 'network' : 'offline');
  }
  if (res.status !== 201) throw await errorOf(res);
  return (await res.json()).id as string;
}

export async function getLoop(id: string): Promise<Shared> {
  let res: Response;
  try {
    res = await fetch(`/api/loops/${encodeURIComponent(id)}`);
  } catch {
    throw new PlanError(navigator.onLine ? 'network' : 'offline');
  }
  if (!res.ok) throw await errorOf(res);
  return (await res.json()) as Shared;
}

export const shareUrl = (id: string) => `${location.origin}/b/${id}`;
export const sharedId = (path = location.pathname) => path.match(/^\/b\/([A-Za-z0-9]{6,32})\/?$/)?.[1] ?? null;
