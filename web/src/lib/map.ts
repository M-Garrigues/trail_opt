// Carte MapLibre appelée directement (I5) : Plan IGN vectoriel + ombrage LiDAR HD.
import * as maplibregl from 'maplibre-gl';
import type { GeoJSONSource, LngLatBoundsLike } from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
// Worker bundlé par Vite (MapLibre 6 le cherche par une URL dynamique que le build ne suit pas)
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';

maplibregl.setWorkerUrl(workerUrl);
import type { Candidate } from './types';
import { Trail3D } from './trail3d';
import { idxAt } from './geo';
import { LM_PATH } from './icons';

export const PLAN_IGN_STYLE = 'https://data.geopf.fr/annexes/ressources/vectorTiles/styles/PLAN.IGN/standard.json';
const SHADOW =
  'https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0' +
  '&LAYER=IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.SHADOW&STYLE=normal&TILEMATRIXSET=PM' +
  '&FORMAT=image/png&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}';

// Spike 3D (D27) : MNT Terrarium de Mapterhorn (IGN RGE ALTI 1 m / LiDAR HD en France, Licence Ouverte 2.0), z ≤ 17, CORS *.
const DEM = 'https://tiles.mapterhorn.com/{z}/{x}/{y}.webp';
const DEM_ATTR = '<a href="https://mapterhorn.com/attribution/">© Mapterhorn</a> · MNT © IGN';
const EXAG = 1.25;
/** Caméra 3D (D57) : inclinaison. */
const PITCH_3D = 68;

/** Couverture (France métropolitaine, D42) : cadrage initial avant chargement de coverage.geojson. */
export const COVERAGE_BOUNDS: LngLatBoundsLike = [[-5.2, 41.3], [9.6, 51.1]];
const WORLD: GeoJSON.Position[] = [[-180, -85], [180, -85], [180, 85], [-180, 85], [-180, -85]];
// n°1 = accent de l'interface (D34) ; carte toujours claire → valeur claire fixe. Ni vert ni brun clair (forêts, courbes IGN).
export const ACCENT = '#a8441c';
export const LOOP_COLORS = [ACCENT, '#1565C0', '#6A1B9A', '#AD1457'];

type Handlers = {
  onClick: (p: { lat: number; lon: number }) => void;
  onLoopClick: (idx: number) => void;
  onLoopHover: (idx: number, p: { lat: number; lng: number } | null) => void;
  onStartDrag: (p: { lat: number; lon: number }) => void;
};

const empty = (): GeoJSON.FeatureCollection => ({ type: 'FeatureCollection', features: [] });

export class TrailMap {
  map: maplibregl.Map;
  private marker: maplibregl.Marker | null = null;
  private ready: Promise<void>;
  private loops: Candidate[] = [];
  /** ligne drapée du tracé choisi : affichée en 2D, et en 3D tant que le ruban n'est pas posé (relief en cours de chargement) */
  private draped = (on: boolean) => { for (const l of ['loops-casing', 'loops-sel']) this.map.setLayoutProperty(l, 'visibility', on ? 'visible' : 'none'); };
  private trail3d = new Trail3D(LOOP_COLORS[0], EXAG, (shown) => { if (this.is3d) this.draped(!shown); });
  private viaMarkers: maplibregl.Marker[] = [];
  private coverageBounds: LngLatBoundsLike = COVERAGE_BOUNDS;
  /** Clics carte ignorés (zone en cours : terra-draw les prend). */
  drawing = false;

  constructor(el: HTMLElement, private h: Handlers, view?: { lat: number; lon: number }, markerLabel = 'Start') {
    this.markerLabel = markerLabel;
    this.map = new maplibregl.Map({
      container: el,
      style: PLAN_IGN_STYLE,
      ...(view ? { center: [view.lon, view.lat] as [number, number], zoom: 12 } : { bounds: COVERAGE_BOUNDS }),
      maxZoom: 18.5,
      maxPitch: 75, // D57 : caméra 3D inclinée à PITCH_3D (60 par défaut dans MapLibre)
      // demande 4 : étiquettes de balisage et d'eau tirées d'OpenStreetMap (ODbL)
      attributionControl: { compact: true, customAttribution: ['© IGN', '<a href="https://www.openstreetmap.org/copyright">© les contributeurs d’OpenStreetMap</a>'] },
      dragRotate: false,
      pitchWithRotate: false,
    });
    this.map.touchZoomRotate.disableRotation();
    // pointeur grossier (mobile) : pincement seulement, pas de boutons zoom
    if (!matchMedia('(pointer: coarse)').matches) this.map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'top-right');
    this.map.addControl(new maplibregl.ScaleControl({ unit: 'metric' }), 'bottom-right');
    this.ready = new Promise((ok) => this.map.once('style.load', () => { this.setup(); ok(); }));
    this.map.on('click', (e) => {
      if (this.drawing) return;
      const lm = this.map.getLayer('landmarks') && this.map.queryRenderedFeatures(e.point, { layers: ['landmarks'] })[0];
      if (lm?.geometry.type === 'Point') { this.map.easeTo({ center: lm.geometry.coordinates as [number, number], duration: 400 }); return; }
      const f = this.map.getLayer('loops-hit') && this.map.queryRenderedFeatures(e.point, { layers: ['loops-hit'] })[0];
      if (f) this.h.onLoopClick(Number(f.properties?.idx));
      else this.h.onClick({ lat: e.lngLat.lat, lon: e.lngLat.lng });
    });
  }
  private markerLabel: string;

  private setup() {
    const m = this.map;
    const layers = m.getStyle().layers;
    const firstSymbol = layers.find((l) => l.type === 'symbol')?.id;
    // demande 5 : nos couches (zone, sorties, repères, curseur) sous les libellés du fond (villes, cols, sommets, lieux-
    // dits), au-dessus de tous ses traits et pictogrammes : 1er libellé texte après le dernier calque non symbole
    const lastShape = layers.length - 1 - [...layers].reverse().findIndex((l) => l.type !== 'symbol');
    const labels = layers.find((l, i) => i > lastShape && l.type === 'symbol' && (l.layout as Record<string, unknown> | undefined)?.['text-field'])?.id;
    // toponymes orographiques du Plan IGN (cols, sommets, lieux-dits de relief) : plus contrastés, halo large
    for (const l of m.getStyle().layers) {
      if (l.type === 'symbol' && l.id.startsWith('toponyme - oro ')) {
        m.setPaintProperty(l.id, 'text-color', '#5a1a14');
        m.setPaintProperty(l.id, 'text-halo-color', 'rgba(255,255,255,0.95)');
        m.setPaintProperty(l.id, 'text-halo-width', 2.5);
      }
    }
    m.addSource('shadow', { type: 'raster', tiles: [SHADOW], tileSize: 256, minzoom: 9, maxzoom: 18, attribution: '© IGN' }); // minzoom : le LiDAR répond 400 aux petits zooms
    m.addLayer({ id: 'shadow', type: 'raster', source: 'shadow', paint: { 'raster-opacity': 0.28 } }, firstSymbol);
    // hors couverture grisé (sous le tracé et les libellés)
    m.addSource('outside', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'outside', type: 'fill', source: 'outside', paint: { 'fill-color': '#455a64', 'fill-opacity': 0.35 } }, firstSymbol);
    m.addSource('zone', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'zone', type: 'line', source: 'zone', paint: { 'line-color': '#37474f', 'line-width': 2, 'line-dasharray': [2, 2] } }, labels);
    m.addSource('loops', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'loops-other', type: 'line', source: 'loops', filter: ['!', ['get', 'sel']],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': '#757575', 'line-width': 3, 'line-opacity': 0.85 } }, labels);
    m.addLayer({ id: 'loops-casing', type: 'line', source: 'loops', filter: ['get', 'sel'],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': '#ffffff', 'line-width': 9 } }, labels);
    m.addLayer({ id: 'loops-sel', type: 'line', source: 'loops', filter: ['get', 'sel'],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': ['get', 'color'], 'line-width': 5 } }, labels);
    m.addLayer({ id: 'loops-hit', type: 'line', source: 'loops', paint: { 'line-color': '#000', 'line-width': 24, 'line-opacity': 0 } }, labels);
    // cols et sommets traversés : pastilles en couche (sous les libellés du fond, contrairement aux marqueurs DOM)
    for (const k of ['summit', 'col'] as const) if (!m.hasImage(`lm-${k}`)) m.addImage(`lm-${k}`, landmarkImage(k), { pixelRatio: 2 });
    m.addSource('landmarks', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'landmarks', type: 'symbol', source: 'landmarks',
      layout: { 'icon-image': ['concat', 'lm-', ['get', 'kind']], 'icon-allow-overlap': true, 'icon-ignore-placement': true } }, labels);
    m.addSource('cursor', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'cursor', type: 'circle', source: 'cursor',
      paint: { 'circle-radius': 7, 'circle-color': '#d62728', 'circle-stroke-color': '#fff', 'circle-stroke-width': 2 } }, labels);
    m.on('mouseenter', 'landmarks', () => { m.getCanvas().style.cursor = 'pointer'; });
    m.on('mouseleave', 'landmarks', () => { m.getCanvas().style.cursor = ''; });
    m.on('mousemove', 'loops-hit', (e) => {
      const f = e.features?.[0];
      if (f && f.properties?.sel) this.h.onLoopHover(Number(f.properties.idx), e.lngLat);
      m.getCanvas().style.cursor = 'pointer';
    });
    m.on('mouseleave', 'loops-hit', () => { this.h.onLoopHover(-1, null); m.getCanvas().style.cursor = ''; });
    m.getContainer().dataset.ready = '1'; // tests e2e : carte et sources prêtes
  }

  /** Grise l'extérieur de la couverture ; fit = recadrer dessus (premier lancement). */
  async setCoverage(g: GeoJSON.Polygon | GeoJSON.MultiPolygon, fit: boolean) {
    await this.ready;
    const outers = (g.type === 'Polygon' ? [g.coordinates] : g.coordinates).map((p) => p[0]);
    const b = new maplibregl.LngLatBounds();
    outers.forEach((r) => r.forEach((c) => b.extend(c as [number, number])));
    this.coverageBounds = b;
    (this.map.getSource('outside') as GeoJSONSource).setData({ type: 'Feature', properties: {},
      geometry: { type: 'Polygon', coordinates: [WORLD, ...outers] } });
    if (fit) this.fitCoverage();
  }

  async setStart(p: { lat: number; lon: number } | null) {
    if (!p) { this.marker?.remove(); this.marker = null; return; }
    if (!this.marker) {
      const el = document.createElement('div');
      el.className = 'start-marker';
      el.setAttribute('aria-label', this.markerLabel);
      this.marker = new maplibregl.Marker({ element: el, draggable: true }).setLngLat([p.lon, p.lat]).addTo(this.map);
      this.marker.on('dragend', () => {
        const ll = this.marker!.getLngLat();
        this.h.onStartDrag({ lat: ll.lat, lon: ll.lng });
      });
    } else this.marker.setLngLat([p.lon, p.lat]);
  }

  async setLoops(cands: Candidate[], sel: number) {
    await this.ready;
    this.loops = cands;
    this.trail3d.setColor(LOOP_COLORS[sel % LOOP_COLORS.length]);
    this.trail3d.setLine(cands[sel] ?? null);
    const features: GeoJSON.Feature[] = cands.map((c, i) => ({
      type: 'Feature',
      properties: { idx: i, sel: i === sel, color: LOOP_COLORS[i % LOOP_COLORS.length] },
      geometry: { type: 'LineString', coordinates: c.lon.map((lo, k) => [lo, c.lat[k]]) },
    }));
    // la boucle choisie dessinée en dernier (au-dessus)
    features.sort((a, b) => Number(a.properties!.sel) - Number(b.properties!.sel));
    (this.map.getSource('loops') as GeoJSONSource).setData({ type: 'FeatureCollection', features });
  }

  async setZone(g: GeoJSON.Geometry | null) {
    await this.ready;
    (this.map.getSource('zone') as GeoJSONSource).setData(g ? { type: 'Feature', properties: {}, geometry: g } : empty());
  }

  async setCursor(c: Candidate | null, i: number) {
    await this.ready;
    const data: GeoJSON.FeatureCollection = c && i >= 0
      ? { type: 'FeatureCollection', features: [{ type: 'Feature', properties: {}, geometry: { type: 'Point', coordinates: [c.lon[i], c.lat[i]] } }] }
      : empty();
    (this.map.getSource('cursor') as GeoJSONSource).setData(data);
  }

  fitLoop(c: Candidate, padding: maplibregl.PaddingOptions, duration = 600) {
    const b = new maplibregl.LngLatBounds();
    c.lon.forEach((lo, i) => b.extend([lo, c.lat[i]]));
    this.map.fitBounds(b, { padding, maxZoom: 16, duration: matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : duration });
  }

  /** Cadre la sortie c (choix d'une variante, D57) : en 2D comme en 3D, avec la marge, animation courte. */
  frameLoop(c: Candidate, padding: maplibregl.PaddingOptions, cap: 'centroid' | 'km1' = 'centroid') {
    if (this.is3d) this.camera3D(c, padding, cap, 500);
    else this.fitLoop(c, padding, 400);
  }

  /** Caméra 3D sur c (D57 : plus près et plus inclinée qu'un simple cadrage, l'itinéraire remplit l'écran). */
  private camera3D(c: Candidate, padding: maplibregl.PaddingOptions, cap: 'centroid' | 'km1', duration: number) {
    const m = this.map, still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const bearing = faceBearing(c, cap);
    const b = new maplibregl.LngLatBounds();
    c.lon.forEach((lo, i) => b.extend([lo, c.lat[i]]));
    const cam = m.cameraForBounds(b, { padding, bearing });
    const dur = still ? 0 : duration;
    // repli : sans cadrage calculable (carte pas encore dimensionnée, rendu logiciel), au moins l'inclinaison
    if (!cam) { m.easeTo({ pitch: PITCH_3D, bearing, duration: dur }); return; }
    // cadrage en perspective : `cameraForBounds` cadre vu de dessus ; inclinée, la sortie ne remplissait plus
    // qu'un cinquième de l'écran. On essaie la vue inclinée (sans l'afficher), on mesure l'emprise projetée
    // de la sortie dans la zone utile (hors marges) et on corrige le zoom pour qu'elle la remplisse (D57).
    let center = maplibregl.LngLat.convert(cam.center ?? m.getCenter()), zoom = cam.zoom ?? 13;
    const start = { center: m.getCenter(), zoom: m.getZoom(), bearing: m.getBearing(), pitch: m.getPitch(), padding: m.getPadding() };
    const cv = m.getCanvas().getBoundingClientRect();
    const w = Math.max(cv.width - (padding.left ?? 0) - (padding.right ?? 0), 50);
    const h = Math.max(cv.height - (padding.top ?? 0) - (padding.bottom ?? 0), 50);
    const step = Math.max(1, Math.floor(c.lat.length / 300));
    for (let k = 0; k < 4; k++) {
      m.jumpTo({ center, zoom, bearing, pitch: PITCH_3D, padding });
      if (k === 0 && !c.lat.every((_, i) => i % step || Number.isFinite(m.project([c.lon[i], c.lat[i]]).x))) break;
      const ps = c.lat.filter((_, i) => i % step === 0).map((la, i) => m.project([c.lon[i * step], la]));
      const x0 = Math.min(...ps.map((p) => p.x)), x1 = Math.max(...ps.map((p) => p.x));
      const y0 = Math.min(...ps.map((p) => p.y)), y1 = Math.max(...ps.map((p) => p.y));
      // centre de l'emprise projetée → centre de la zone utile (en perspective, le cadrage vu de dessus la
      // poussait hors de l'écran sur téléphone), puis échelle avec 10 % de marge, pas borné
      center = m.unproject([(x0 + x1) / 2, (y0 + y1) / 2]);
      zoom += Math.max(-1, Math.min(1, Math.log2(0.9 * Math.min(w / Math.max(x1 - x0, 1), h / Math.max(y1 - y0, 1)))));
    }
    m.jumpTo(start);
    // projection inexploitable (NaN : relief pas encore chargé) : cadrage vu de dessus, incliné
    const ok = Number.isFinite(zoom) && Number.isFinite(center.lng) && Number.isFinite(center.lat);
    if (!ok) [center, zoom] = [maplibregl.LngLat.convert(cam.center ?? start.center), cam.zoom ?? start.zoom];
    m.easeTo({ center, zoom: Math.min(zoom, 17), bearing, pitch: PITCH_3D, padding, duration: dur });
  }

  /** Centre sur p ; offsetY > 0 remonte le point (au-dessus de la bottom sheet). */
  flyTo(p: { lat: number; lon: number }, zoom = 14, offsetX = 0, offsetY = 0) {
    this.map.easeTo({ center: [p.lon, p.lat], zoom: Math.max(zoom, this.map.getZoom()), offset: [offsetX, -offsetY], duration: 0 });
  }

  /** Centre (animé) sur le point i du tracé ; offsetY > 0 remonte le point au-dessus de la feuille / du profil. */
  centerOn(c: Candidate, i: number, offsetY = 0) {
    const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    this.map.easeTo({ center: [c.lon[i], c.lat[i]], offset: [0, -offsetY], duration: still ? 0 : 500 });
  }

  is3d = false;
  /** Bascule 2D/3D. En 3D avec une boucle : caméra « face au parcours » depuis le départ. */
  async set3D(on: boolean, c: Candidate | null, padding: maplibregl.PaddingOptions, cap: 'centroid' | 'km1' = 'centroid') {
    await this.ready;
    const m = this.map, still = matchMedia('(prefers-reduced-motion: reduce)').matches;
    this.is3d = on;
    if (on && !m.getSource('dem')) {
      m.addSource('dem', { type: 'raster-dem', tiles: [DEM], tileSize: 512, maxzoom: 17, encoding: 'terrarium', attribution: DEM_ATTR });
      m.setSky({ 'sky-color': '#bcd7ec', 'horizon-color': '#eef2f0', 'fog-color': '#f2efe9', 'sky-horizon-blend': 0.5, 'horizon-fog-blend': 0.6, 'fog-ground-blend': 0.7 });
    }
    m.setTerrain(on ? { source: 'dem', exaggeration: EXAG } : null);
    // tracé choisi : ligne drapée en 2D ; en 3D, ruban surélevé dès que le relief est chargé (trail3d.ts)
    if (!on) this.draped(true);
    if (on && !m.getLayer(this.trail3d.id)) m.addLayer(this.trail3d, 'landmarks');
    if (!on && m.getLayer(this.trail3d.id)) m.removeLayer(this.trail3d.id);
    if (on) { m.dragRotate.enable(); m.touchZoomRotate.enableRotation(); m.touchPitch.enable(); }
    else { m.dragRotate.disable(); m.touchZoomRotate.disableRotation(); m.touchPitch.disable(); }
    const duration = still ? 0 : 1200;
    if (!on) { m.easeTo({ pitch: 0, bearing: 0, duration }); return; }
    if (!c) { m.easeTo({ pitch: PITCH_3D, duration }); return; }
    this.camera3D(c, padding, cap, duration);
  }

  /** Points de passage : repères numérotés déplaçables (D34). */
  setVia(pts: { lat: number; lon: number }[], label: (n: number) => string, onDrag: (i: number, p: { lat: number; lon: number }) => void) {
    this.viaMarkers.forEach((mk) => mk.remove());
    this.viaMarkers = pts.map((p, i) => {
      const el = document.createElement('div');
      el.className = 'via-marker';
      el.textContent = String(i + 1);
      el.setAttribute('aria-label', label(i + 1));
      el.addEventListener('click', () => this.map.easeTo({ center: [p.lon, p.lat], duration: 400 }));
      const mk = new maplibregl.Marker({ element: el, draggable: true }).setLngLat([p.lon, p.lat]).addTo(this.map);
      mk.on('dragend', () => { const ll = mk.getLngLat(); onDrag(i, { lat: ll.lat, lon: ll.lng }); });
      return mk;
    });
  }

  /** Cols et sommets traversés (repères, D34) : couche `landmarks` ; au clavier, la liste du détail centre la carte. */
  async setLandmarks(c: Candidate | null) {
    await this.ready;
    const features = (c?.landmarks ?? []).map((l) => {
      const i = idxAt(c!, l.dist_m);
      return { type: 'Feature' as const, properties: { kind: l.kind, name: l.name }, geometry: { type: 'Point' as const, coordinates: [c!.lon[i], c!.lat[i]] } };
    });
    (this.map.getSource('landmarks') as GeoJSONSource).setData({ type: 'FeatureCollection', features });
  }

  fitCoverage() {
    this.map.fitBounds(this.coverageBounds, { padding: 20, duration: 0 });
  }
}

/** Cap (degrés, 0 = nord) du départ vers le centroïde de la boucle, ou vers le point à 1 km. */
export function faceBearing(c: Candidate, cap: 'centroid' | 'km1' = 'centroid'): number {
  let lat = 0, lon = 0;
  if (cap === 'km1') {
    const i = Math.max(1, c.dist.findIndex((d) => d >= 1000));
    lat = c.lat[i]; lon = c.lon[i];
  } else {
    lat = c.lat.reduce((s, v) => s + v, 0) / c.lat.length;
    lon = c.lon.reduce((s, v) => s + v, 0) / c.lon.length;
  }
  const k = Math.cos((c.lat[0] * Math.PI) / 180);
  return (Math.atan2((lon - c.lon[0]) * k, lat - c.lat[0]) * 180) / Math.PI;
}

/** Pastille de repère (carte toujours claire) : disque papier bordé, pictogramme au trait, 2× pour l'écran Retina. */
function landmarkImage(kind: keyof typeof LM_PATH): ImageData {
  const n = 52, c = document.createElement('canvas');
  c.width = c.height = n;
  const g = c.getContext('2d')!;
  const ink = kind === 'col' ? '#6b6b6b' : '#1d1d1d';
  g.beginPath(); g.arc(n / 2, n / 2, n / 2 - 3, 0, 2 * Math.PI);
  g.fillStyle = '#ffffff'; g.fill(); g.lineWidth = 3; g.strokeStyle = ink; g.stroke();
  g.translate(n / 2 - 18, n / 2 - 18); g.scale(1.8, 1.8);
  g.lineWidth = 1.8; g.lineCap = 'round'; g.lineJoin = 'round'; g.strokeStyle = ink;
  g.stroke(new Path2D(LM_PATH[kind]));
  return g.getImageData(0, 0, n, n);
}
