// GPX généré côté client (D11). Nom neutre en langue : « optrail 10.2 km +433 m ».
import type { Candidate } from './types';

const esc = (s: string) => s.replace(/[<>&"]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;' })[c]!);

export function loopName(c: Pick<Candidate, 'length_m' | 'dplus_m'>): string {
  return `optrail ${(c.length_m / 1000).toFixed(1)} km +${Math.round(c.dplus_m)} m`;
}

export function gpxFilename(c: Pick<Candidate, 'length_m' | 'dplus_m'>): string {
  return `optrail-${(c.length_m / 1000).toFixed(1)}km-${Math.round(c.dplus_m)}m.gpx`;
}

export function toGpx(c: Candidate, dataVersion = ''): string {
  const name = esc(loopName(c));
  const pts = c.lat
    .map((la, i) => `<trkpt lat="${la.toFixed(6)}" lon="${c.lon[i].toFixed(6)}"><ele>${c.ele[i].toFixed(1)}</ele></trkpt>`)
    .join('\n');
  return `<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="optrail" xmlns="http://www.topografix.com/GPX/1/1">
<metadata><name>${name}</name><desc>Données © IGN BD TOPO®, RGE ALTI® (Etalab 2.0) ${esc(dataVersion)}</desc><link href="https://optrail.eu"><text>optrail</text></link></metadata>
<trk><name>${name}</name><type>trail_running</type><trkseg>
${pts}
</trkseg></trk>
</gpx>
`;
}

export function gpxFile(c: Candidate, dataVersion = ''): File {
  return new File([toGpx(c, dataVersion)], gpxFilename(c), { type: 'application/gpx+xml' });
}

export function downloadGpx(c: Candidate, dataVersion = ''): void {
  const url = URL.createObjectURL(gpxFile(c, dataVersion));
  const a = Object.assign(document.createElement('a'), { href: url, download: gpxFilename(c) });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** GPX = seule sortie v1 (D27) : partage de fichier si le système le permet (mobile), sinon téléchargement. */
export const canShareGpx = (c: Candidate) =>
  typeof navigator.canShare === 'function' && navigator.canShare({ files: [gpxFile(c)] });
export async function saveGpx(c: Candidate, dataVersion = ''): Promise<void> {
  if (!canShareGpx(c)) { downloadGpx(c, dataVersion); return; }
  const f = gpxFile(c, dataVersion);
  try { await navigator.share({ files: [f], title: f.name }); } catch { /* annulé */ }
}
