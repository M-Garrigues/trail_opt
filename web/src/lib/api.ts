import type { PlanResponse, Msg } from './types';

export class PlanError extends Error {
  constructor(public code: string, public params: Msg['params'] = {}, public status = 0) {
    super(code);
  }
}

/** GET /api/plan ; toute erreur devient un PlanError avec un code D14 (429 du coupe-circuit/quota → service_paused, D32 B2). */
export async function fetchPlan(q: URLSearchParams, token: string | null, signal: AbortSignal): Promise<PlanResponse> {
  let res: Response;
  try {
    res = await fetch(`/api/plan?${q}`, { signal, headers: token ? { 'X-Turnstile-Token': token } : {} });
  } catch (e) {
    if (signal.aborted) throw new PlanError('cancelled');
    throw new PlanError(navigator.onLine ? 'network' : 'offline');
  }
  if (res.status === 429) throw new PlanError('service_paused', {}, 429);
  let body: unknown = null;
  try {
    body = await res.json();
  } catch {
    if (signal.aborted) throw new PlanError('cancelled');
  }
  if (res.ok && body && typeof body === 'object' && 'candidates' in body) return body as PlanResponse;
  const err = (body as { error?: Msg } | null)?.error;
  if (err?.code) throw new PlanError(err.code, err.params ?? {}, res.status);
  throw new PlanError(`http_${res.status}`, {}, res.status);
}

/** GET /api/plan?...&diagnose=1 (D46) : réglage qui atteindrait la cible (`suggest`) / rendrait assez de boucles (`fewer_suggest`). Échec silencieux : null. */
export async function fetchDiagnose(q: URLSearchParams, token: string | null, signal: AbortSignal): Promise<{ suggest?: Msg['suggest']; fewer_suggest?: Msg['suggest'] } | null> {
  try {
    const d = new URLSearchParams(q);
    d.set('diagnose', '1');
    const res = await fetch(`/api/plan?${d}`, { signal, headers: token ? { 'X-Turnstile-Token': token } : {} });
    return res.ok ? await res.json() : null;
  } catch {
    return null;
  }
}
