// localStorage (jamais de cookie, D22/D24) : tout est enveloppé, l'app tourne sans.
const PREFIX = 'optrail.';

export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    return raw == null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

/** Écrit ; lève l'erreur d'origine (QuotaExceededError…) pour qui veut la traiter. */
export function saveOrThrow(key: string, value: unknown): void {
  localStorage.setItem(PREFIX + key, JSON.stringify(value));
}

export function save(key: string, value: unknown): boolean {
  try {
    saveOrThrow(key, value);
    return true;
  } catch {
    return false;
  }
}

export function remove(key: string): void {
  try {
    localStorage.removeItem(PREFIX + key);
  } catch {
    /* indisponible */
  }
}

export function clearAll(): void {
  try {
    for (const k of Object.keys(localStorage)) if (k.startsWith(PREFIX)) localStorage.removeItem(k);
  } catch {
    /* indisponible */
  }
}
