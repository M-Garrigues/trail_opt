// Carte MapLibre appelée directement (I5) : Plan IGN vectoriel + ombrage LiDAR HD (port de trailopt/maplayers.py).
import * as maplibregl from 'maplibre-gl';
import type { GeoJSONSource, LngLatBoundsLike } from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
// Worker bundlé par Vite (MapLibre 6 le cherche par une URL dynamique que le build ne suit pas)
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';

maplibregl.setWorkerUrl(workerUrl);
import type { Candidate } from './types';

export const PLAN_IGN_STYLE = 'https://data.geopf.fr/annexes/ressources/vectorTiles/styles/PLAN.IGN/standard.json';
const SHADOW =
  'https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0' +
  '&LAYER=IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.SHADOW&STYLE=normal&TILEMATRIXSET=PM' +
  '&FORMAT=image/png&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}';

// Spike 3D (D27) : MNT Terrarium de Mapterhorn (IGN RGE ALTI 1 m / LiDAR HD en France, Licence Ouverte 2.0), z ≤ 17, CORS *.
const DEM = 'https://tiles.mapterhorn.com/{z}/{x}/{y}.webp';
const DEM_ATTR = '<a href="https://mapterhorn.com/attribution/">© Mapterhorn</a> · MNT © IGN';

/** Couverture v1 (IdF + Isère) : cadrage initial avant chargement de coverage.geojson. */
export const COVERAGE_BOUNDS: LngLatBoundsLike = [[1.4, 44.65], [6.4, 49.25]];
const WORLD: GeoJSON.Position[] = [[-180, -85], [180, -85], [180, 85], [-180, 85], [-180, -85]];
export const LOOP_COLORS = ['#C2185B', '#1565C0', '#E65100', '#6A1B9A'];

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
    this.map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'top-right');
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
    m.addSource('shadow', { type: 'raster', tiles: [SHADOW], tileSize: 256, maxzoom: 18, attribution: '© IGN' });
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
    m.setTerrain(on ? { source: 'dem', exaggeration: 1.25 } : null);
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
