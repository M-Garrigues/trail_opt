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

/** Couverture v1 (IdF + Isère) : cadrage initial avant chargement de coverage.geojson. */
export const COVERAGE_BOUNDS: LngLatBoundsLike = [[1.4, 44.65], [6.4, 49.25]];
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
  private marks: maplibregl.Marker[] = [];
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
      attributionControl: { compact: true, customAttribution: '© IGN' },
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
      const f = this.map.getLayer('loops-hit') && this.map.queryRenderedFeatures(e.point, { layers: ['loops-hit'] })[0];
      if (f) this.h.onLoopClick(Number(f.properties?.idx));
      else this.h.onClick({ lat: e.lngLat.lat, lon: e.lngLat.lng });
    });
  }
  private markerLabel: string;

  private setup() {
    const m = this.map;
    const firstSymbol = m.getStyle().layers.find((l) => l.type === 'symbol')?.id;
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
    m.addLayer({ id: 'zone', type: 'line', source: 'zone', paint: { 'line-color': '#37474f', 'line-width': 2, 'line-dasharray': [2, 2] } });
    m.addSource('loops', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'loops-other', type: 'line', source: 'loops', filter: ['!', ['get', 'sel']],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': '#757575', 'line-width': 3, 'line-opacity': 0.85 } });
    m.addLayer({ id: 'loops-casing', type: 'line', source: 'loops', filter: ['get', 'sel'],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': '#ffffff', 'line-width': 9 } });
    m.addLayer({ id: 'loops-sel', type: 'line', source: 'loops', filter: ['get', 'sel'],
      layout: { 'line-join': 'round', 'line-cap': 'round' }, paint: { 'line-color': ['get', 'color'], 'line-width': 5 } });
    m.addLayer({ id: 'loops-hit', type: 'line', source: 'loops', paint: { 'line-color': '#000', 'line-width': 24, 'line-opacity': 0 } });
    m.addSource('cursor', { type: 'geojson', data: empty() });
    m.addLayer({ id: 'cursor', type: 'circle', source: 'cursor',
      paint: { 'circle-radius': 7, 'circle-color': '#d62728', 'circle-stroke-color': '#fff', 'circle-stroke-width': 2 } });
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

  fitLoop(c: Candidate, padding: maplibregl.PaddingOptions) {
    const b = new maplibregl.LngLatBounds();
    c.lon.forEach((lo, i) => b.extend([lo, c.lat[i]]));
    this.map.fitBounds(b, { padding, maxZoom: 16, duration: matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 600 });
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
    if (on && !m.getLayer(this.trail3d.id)) m.addLayer(this.trail3d, 'cursor');
    if (!on && m.getLayer(this.trail3d.id)) m.removeLayer(this.trail3d.id);
    if (on) { m.dragRotate.enable(); m.touchZoomRotate.enableRotation(); m.touchPitch.enable(); }
    else { m.dragRotate.disable(); m.touchZoomRotate.disableRotation(); m.touchPitch.disable(); }
    const duration = still ? 0 : 1200;
    if (!on) { m.easeTo({ pitch: 0, bearing: 0, duration }); return; }
    if (!c) { m.easeTo({ pitch: 60, duration }); return; }
    const bearing = faceBearing(c, cap);
    const b = new maplibregl.LngLatBounds();
    c.lon.forEach((lo, i) => b.extend([lo, c.lat[i]]));
    const cam = m.cameraForBounds(b, { padding, bearing });
    // même zoom qu'en 2D : en perspective l'avant grossit, le fond rapetisse, la boucle reste dans le cadre
    if (cam) m.easeTo({ center: cam.center, zoom: cam.zoom ?? 13, bearing, pitch: 60, padding, duration });
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

  /** Cols et sommets traversés (repères, D34). */
  setLandmarks(c: Candidate | null) {
    this.marks.forEach((mk) => mk.remove());
    this.marks = (c?.landmarks ?? []).map((l) => {
      const el = document.createElement('div');
      const i = idxAt(c!, l.dist_m);
      el.className = `landmark ${l.kind}`;
      el.innerHTML = `<svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="${LM_PATH[l.kind]}" /></svg>`;
      el.title = l.name + (l.ele_m != null ? ` · ≈ ${Math.round(l.ele_m / 10) * 10} m` : '');
      el.setAttribute('role', 'button');
      el.setAttribute('aria-label', el.title);
      el.tabIndex = 0;
      const go = () => this.map.easeTo({ center: [c!.lon[i], c!.lat[i]], duration: 400 });
      el.addEventListener('click', (ev) => { ev.stopPropagation(); go(); });
      el.addEventListener('keydown', (ev) => { if (ev.key === 'Enter') go(); });
      return new maplibregl.Marker({ element: el }).setLngLat([c!.lon[i], c!.lat[i]]).addTo(this.map);
    });
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
