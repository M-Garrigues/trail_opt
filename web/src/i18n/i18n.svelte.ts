import { dicts, type Lang } from './format';
import { load, save } from '../lib/store';

function detect(): Lang {
  const q = new URLSearchParams(location.search).get('lang');
  if (q === 'fr' || q === 'en') {
    save('lang', q);
    return q;
  }
  const s = load<string | null>('lang', null);
  if (s === 'fr' || s === 'en') return s;
  for (const l of navigator.languages ?? []) {
    if (l.startsWith('fr')) return 'fr';
    if (l.startsWith('en')) return 'en';
  }
  return 'en';
}

export const i18n = $state({ lang: detect() });

export const t = () => dicts[i18n.lang];

export function setLang(l: Lang) {
  i18n.lang = l;
  save('lang', l);
  document.documentElement.lang = l;
}
document.documentElement.lang = i18n.lang;
