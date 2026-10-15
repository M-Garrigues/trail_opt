// Sport courant (D70) : `trail` ou `bike`. Ordre de lecture : ?sport= > localStorage > trail. Un `?sport=` valide est
// mémorisé puis retiré de l'URL (un rechargement après bascule ne doit pas revenir en arrière). `<html data-sport>`
// pilote l'accent (app.css) ; les textes (`t()`) et la carte lisent aussi `sport.id`.
import { load, save } from './store';
import type { Sport } from './catalog'; // type seulement : catalog → format → ce module formerait un cycle

const isSport = (x: unknown): x is Sport => x === 'trail' || x === 'bike';

function initial(): Sport {
  if (typeof location !== 'undefined') {
    const p = new URLSearchParams(location.search);
    const q = p.get('sport');
    if (isSport(q)) {
      save('sport', q);
      p.delete('sport');
      history.replaceState(history.state, '', location.pathname + (p.size ? `?${p}` : '') + location.hash);
      return q;
    }
  }
  const s = load<unknown>('sport', 'trail');
  return isSport(s) ? s : 'trail';
}

export const sport = $state({ id: initial() });
if (typeof document !== 'undefined') document.documentElement.dataset.sport = sport.id;

/** Change de sport partout (état, mémoire, accent). Les réglages et la sortie affichée : `setSport` (app.svelte.ts). */
export function useSport(id: Sport) {
  sport.id = id;
  save('sport', id);
  if (typeof document !== 'undefined') document.documentElement.dataset.sport = id;
}
