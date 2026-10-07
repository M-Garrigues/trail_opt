<script lang="ts">
  // Carte de densité des départs (cellules ~500 m) ; cercles rouges : demandes hors couverture.
  import * as maplibregl from 'maplibre-gl';
  import { onMount } from 'svelte';
  import { PLAN_IGN_STYLE, COVERAGE_BOUNDS } from '../lib/map';
  import type { Cell } from './stats';

  let { starts, outside }: { starts: Cell[]; outside: Cell[] } = $props();
  let el: HTMLDivElement;
  let map: maplibregl.Map | undefined;

  const fc = (cells: Cell[]): GeoJSON.FeatureCollection => ({
    type: 'FeatureCollection',
    features: cells.map((c) => ({ type: 'Feature', properties: { n: c.n }, geometry: { type: 'Point', coordinates: [c.lon, c.lat] } })),
  });

  function update() {
    if (!map?.isStyleLoaded()) return;
    (map.getSource('starts') as maplibregl.GeoJSONSource | undefined)?.setData(fc(starts));
    (map.getSource('outside') as maplibregl.GeoJSONSource | undefined)?.setData(fc(outside));
    const all = [...starts, ...outside];
    if (all.length) {
      const b = new maplibregl.LngLatBounds();
      for (const c of all) b.extend([c.lon, c.lat]);
      map.fitBounds(b, { padding: 40, maxZoom: 11, duration: 0 });
    }
  }

  onMount(() => {
    map = new maplibregl.Map({ container: el, style: PLAN_IGN_STYLE, bounds: COVERAGE_BOUNDS, attributionControl: { compact: true, customAttribution: '© IGN' } });
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'top-right');
    map.once('style.load', () => {
      map!.addSource('starts', { type: 'geojson', data: fc([]) });
      map!.addSource('outside', { type: 'geojson', data: fc([]) });
      map!.addLayer({
        id: 'heat', type: 'heatmap', source: 'starts',
        paint: {
          'heatmap-weight': ['interpolate', ['linear'], ['get', 'n'], 1, 0.4, 20, 1],
          'heatmap-radius': ['interpolate', ['linear'], ['zoom'], 5, 12, 12, 30],
          'heatmap-opacity': 0.85,
        },
      });
      map!.addLayer({
        id: 'outside', type: 'circle', source: 'outside',
        paint: { 'circle-radius': ['interpolate', ['linear'], ['get', 'n'], 1, 5, 20, 12], 'circle-color': '#d32f2f', 'circle-opacity': 0.6, 'circle-stroke-color': '#fff', 'circle-stroke-width': 1 },
      });
      update();
    });
    return () => map?.remove();
  });

  $effect(() => { void [starts, outside]; update(); });
</script>

<div class="map" bind:this={el} role="img" aria-label="Carte de densité des départs"></div>

<style>
  .map { height: 420px; border-radius: 10px; overflow: hidden; border: 1px solid var(--border); }
</style>
