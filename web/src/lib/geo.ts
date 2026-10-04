// Géométrie légère côté client (I4 : pas de turf).
const R = 6371000;

/** Pente (%) en chaque point, mesurée sur ~50 m (port de ProfileLink). dist en m. */
export function grades(dist: number[], ele: number[]): number[] {
  const n = dist.length, G: number[] = [];
  let i0 = 0, i1 = 0;
  for (let i = 0; i < n; i++) {
    while (i0 < i && dist[i] - dist[i0] > 25) i0++;
    while (i1 < n - 1 && dist[i1] - dist[i] < 25) i1++;
    const run = dist[i1] - dist[i0];
    G.push(run > 1 ? (100 * (ele[i1] - ele[i0])) / run : 0);
  }
  return G;
}

/** Indice du point le plus proche d'une distance cumulée (recherche binaire). */
export function indexAtDist(dist: number[], d: number): number {
  let lo = 0, hi = dist.length - 1;
  while (hi - lo > 1) {
    const m = (lo + hi) >> 1;
    if (dist[m] < d) lo = m;
    else hi = m;
  }
  return Math.abs(dist[lo] - d) <= Math.abs(dist[hi] - d) ? lo : hi;
}

export function nearestIndex(lat: number[], lon: number[], p: { lat: number; lng: number }): number {
  let best = 0, bd = Infinity;
  const k = Math.cos((p.lat * Math.PI) / 180);
  for (let i = 0; i < lat.length; i++) {
    const dy = lat[i] - p.lat, dx = (lon[i] - p.lng) * k, q = dx * dx + dy * dy;
    if (q < bd) { bd = q; best = i; }
  }
  return best;
}

/** Point dans un polygone (lon, lat), lancer de rayon. */
export function inPolygon(pt: [number, number], ring: [number, number][]): boolean {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i], [xj, yj] = ring[j];
    if (yi > pt[1] !== yj > pt[1] && pt[0] < ((xj - xi) * (pt[1] - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** Point (lon, lat) dans un Polygon / MultiPolygon GeoJSON (trous compris). */
export function inGeometry(pt: [number, number], g: GeoJSON.Polygon | GeoJSON.MultiPolygon): boolean {
  const polys = g.type === 'Polygon' ? [g.coordinates] : g.coordinates;
  return polys.some(([outer, ...holes]) =>
    inPolygon(pt, outer as [number, number][]) && !holes.some((h) => inPolygon(pt, h as [number, number][])));
}

/** Douglas–Peucker en plan local (m) ; renvoie les indices gardés. */
export function simplify(lat: number[], lon: number[], tol: number): number[] {
  const n = lat.length;
  if (n < 3) return [...Array(n).keys()];
  const k = Math.cos((lat[0] * Math.PI) / 180), f = (Math.PI / 180) * R;
  const x = lon.map((v) => v * f * k), y = lat.map((v) => v * f);
  const keep = new Uint8Array(n);
  keep[0] = keep[n - 1] = 1;
  const stack: [number, number][] = [[0, n - 1]];
  while (stack.length) {
    const [a, b] = stack.pop()!;
    const dx = x[b] - x[a], dy = y[b] - y[a], L = Math.hypot(dx, dy);
    let dmax = 0, idx = -1;
    for (let i = a + 1; i < b; i++) {
      const d = L > 0 ? Math.abs(dy * x[i] - dx * y[i] + x[b] * y[a] - y[b] * x[a]) / L : Math.hypot(x[i] - x[a], y[i] - y[a]);
      if (d > dmax) { dmax = d; idx = i; }
    }
    if (dmax > tol && idx > 0) {
      keep[idx] = 1;
      stack.push([a, idx], [idx, b]);
    }
  }
  return [...keep.keys()].filter((i) => keep[i]);
}

/** Départ mémorisé arrondi à 0,01° (~1 km, D22) : sert seulement à cadrer la carte. */
export const roundStart = (p: { lat: number; lon: number }) => ({
  lat: Math.round(p.lat * 100) / 100,
  lon: Math.round(p.lon * 100) / 100,
});

/** « lat, lon » saisi dans la recherche. */
export function parseLatLon(s: string): { lat: number; lon: number } | null {
  const m = s.trim().match(/^(-?\d+(?:[.,]\d+)?)\s*[,;\s]\s*(-?\d+(?:[.,]\d+)?)$/);
  if (!m) return null;
  const lat = parseFloat(m[1].replace(',', '.')), lon = parseFloat(m[2].replace(',', '.'));
  return Math.abs(lat) <= 90 && Math.abs(lon) <= 180 ? { lat, lon } : null;
}
