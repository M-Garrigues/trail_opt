// Installation de l'app (D12 : la PWA installée EST l'app mobile, pas de store en v1).
// Android / Chrome / Edge : événement `beforeinstallprompt` retenu puis rejoué au clic (aucun service worker requis).
// iOS : pas d'API → consigne « Partager → Sur l'écran d'accueil ». Déjà installée : rien.
import { load, save } from './store';

export type Platform = 'ios' | 'android' | 'desktop';
/** iPadOS se présente comme un Mac : seul l'écran tactile le distingue. */
export function platform(ua: string, touchPoints = 0): Platform {
  if (/iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && touchPoints > 1)) return 'ios';
  return /Android/.test(ua) ? 'android' : 'desktop';
}

/** Ce qu'on peut proposer : rien, l'invite du navigateur, la consigne iOS, la consigne générique (Android sans invite). */
export type How = 'none' | 'prompt' | 'ios' | 'manual';
export function how(p: Platform, standalone: boolean, canPrompt: boolean): How {
  if (standalone) return 'none';
  if (canPrompt) return 'prompt';
  return p === 'ios' ? 'ios' : p === 'android' ? 'manual' : 'none';
}

/** Suggestion spontanée : une seule fois, téléphone/tablette seulement, et seulement si le geste est sûr d'aboutir. */
export const shouldHint = (h: How, p: Platform, seen: boolean) => !seen && (h === 'ios' || (h === 'prompt' && p !== 'desktop'));

type PromptEvent = Event & { prompt(): Promise<unknown> };
const web = typeof window !== 'undefined';
const here: Platform = web ? platform(navigator.userAgent, navigator.maxTouchPoints) : 'desktop';

export const install = $state({
  prompt: null as PromptEvent | null,
  standalone: web && (matchMedia('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true),
  hint: false,
});
export const installHow = () => how(here, install.standalone, !!install.prompt);

if (web) {
  // preventDefault : pas de bandeau du navigateur au premier chargement, c'est nous qui choisissons le moment
  addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); install.prompt = e as PromptEvent; });
  addEventListener('appinstalled', () => { install.prompt = null; install.hint = false; install.standalone = true; });
}

/** Itinéraires générés avant de suggérer l'installation (compteur persistant, cumulable entre visites). */
export const HINT_AFTER = 3;
/** À appeler à chaque calcul réussi (pas pour une boucle rouverte) : au 3e, affiche la suggestion, au plus une fois par appareil. */
export function routeGenerated() {
  const n = load('installRuns', 0) + 1;
  save('installRuns', n);
  if (n < HINT_AFTER || !shouldHint(installHow(), here, load('install', false))) return;
  install.hint = true;
  save('install', true);
}
export function dismissHint() { install.hint = false; }

/** Invite du navigateur ; l'événement ne sert qu'une fois (refus → consigne générique dans le menu). */
export async function promptInstall() {
  const e = install.prompt;
  install.prompt = null;
  install.hint = false;
  await e?.prompt();
}
