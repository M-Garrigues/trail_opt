// Cloudflare Turnstile invisible (D8) : jeton pris au clic « Trouver ma boucle », envoyé en X-Turnstile-Token.
// Clé de site : VITE_TURNSTILE_SITEKEY, sinon clé de TEST Cloudflare invisible qui passe toujours
// (1x00000000000000000000BB ; variante visible : …AA, cf. scripts/dev.sh).
const SITEKEY = import.meta.env.VITE_TURNSTILE_SITEKEY || '1x00000000000000000000BB';

type TS = {
  render: (el: HTMLElement, o: Record<string, unknown>) => string;
  execute: (id: string) => void;
  reset: (id: string) => void;
};
declare global { interface Window { turnstile?: TS } }

let loading: Promise<TS | null> | null = null;
function loadScript(): Promise<TS | null> {
  loading ??= new Promise((ok) => {
    const s = document.createElement('script');
    s.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
    s.async = true;
    s.onload = () => ok(window.turnstile ?? null);
    s.onerror = () => ok(null);
    document.head.append(s);
  });
  return loading;
}

let widget: string | null = null;
let pending: ((t: string | null) => void) | null = null;

/** Jeton à usage unique ; null si Turnstile est injoignable (le serveur répondra bot_check_failed en prod). */
export async function getToken(timeoutMs = 8000): Promise<string | null> {
  const ts = await loadScript();
  if (!ts) return null;
  if (widget == null) {
    const el = document.createElement('div');
    el.className = 'turnstile';
    document.body.append(el);
    widget = ts.render(el, {
      sitekey: SITEKEY,
      appearance: 'interaction-only',
      execution: 'execute',
      callback: (t: string) => pending?.(t),
      'error-callback': () => pending?.(null),
    });
  } else ts.reset(widget);
  return new Promise((ok) => {
    const timer = setTimeout(() => done(null), timeoutMs);
    const done = (t: string | null) => { clearTimeout(timer); pending = null; ok(t); };
    pending = done;
    ts.execute(widget!);
  });
}
