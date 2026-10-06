<script lang="ts">
  // E4 zone : polygone terra-draw, 3 à 50 sommets. Départ hors zone : indice seulement, la recherche le refusera. Monté = mode dessin ; démonté = carte rendue.
  // Une fois fermée (ou rouverte), la zone reste éditable : glisser un sommet, glisser un point milieu pour en ajouter.
  import { onMount } from 'svelte';
  import type { Map as MlMap } from 'maplibre-gl';
  import { TerraDraw, TerraDrawPolygonMode, TerraDrawSelectMode, TerraDrawModeUndoRedo } from 'terra-draw';
  import { TerraDrawMapLibreGLAdapter } from 'terra-draw-maplibre-gl-adapter';
  import { app, close } from '../lib/app.svelte';
  import { inPolygon } from '../lib/geo';
  import { t } from '../i18n/i18n.svelte';
  import type { TrailMap } from '../lib/map';

  let { tmap }: { tmap: TrailMap } = $props();
  const MAX = 50;
  let draw: TerraDraw | null = null;
  let ring = $state<[number, number][] | null>(app.zone ? [...app.zone] : null); // polygone fermé (sans répétition)
  let points = $state(0); // sommets posés pendant le dessin
  let msg = $state('');

  const valid = $derived(!!ring && ring.length >= 3 && ring.length <= MAX);
  // indice non bloquant : la zone se valide quand même, l'erreur vient au lancement de la recherche
  const outside = $derived(valid && !!app.start && !inPolygon([app.start.lon, app.start.lat], ring!));

  const drawing = () => draw?.getSnapshot().find((x) => x.geometry.type === 'Polygon');
  function readSnapshot() {
    const f = drawing();
    return f ? ((f.geometry as GeoJSON.Polygon).coordinates[0] as [number, number][]) : null;
  }

  onMount(() => {
    const map = tmap.map as MlMap;
    tmap.drawing = true;
    // anneau fermé : sommets + répétition du premier
    const maxRing = (f: GeoJSON.Feature) => ({ valid: (f.geometry as GeoJSON.Polygon).coordinates[0].length <= MAX + 1, reason: 'max' });
    draw = new TerraDraw({
      adapter: new TerraDrawMapLibreGLAdapter({ map }),
      modes: [
        new TerraDrawPolygonMode({
          // anneau en cours = sommets + curseur + fermeture : 50 sommets max
          validation: (f) => ({ valid: (f.geometry as GeoJSON.Polygon).coordinates[0].length <= MAX + 2, reason: 'max' }),
          styles: { fillColor: '#1565C0', fillOpacity: 0.12, outlineColor: '#1565C0', outlineWidth: 3,
            closingPointColor: '#ffffff', closingPointOutlineColor: '#1565C0', closingPointWidth: 7, closingPointOutlineWidth: 3 },
        }),
        new TerraDrawSelectMode({
          flags: { polygon: { feature: { validation: maxRing, coordinates: { draggable: true, midpoints: true } } } },
          allowManualDeselection: false,
          styles: { selectedPolygonColor: '#1565C0', selectedPolygonFillOpacity: 0.12, selectedPolygonOutlineColor: '#1565C0',
            selectedPolygonOutlineWidth: 3, selectionPointColor: '#ffffff', selectionPointOutlineColor: '#1565C0',
            selectionPointWidth: 7, selectionPointOutlineWidth: 3, midPointColor: '#1565C0', midPointOutlineColor: '#ffffff', midPointWidth: 4 },
        }),
      ],
      undoRedo: { modeLevel: new TerraDrawModeUndoRedo() },
    });
    draw.start();
    draw.setMode('polygon');
    if (ring) {
      const [v] = draw.addFeatures([{ type: 'Feature', properties: { mode: 'polygon' },
        geometry: { type: 'Polygon', coordinates: [[...ring, ring[0]]] } }]);
      if (v.id != null) edit(v.id);
    }
    draw.on('change', () => {
      const r = readSnapshot();
      if (ring && r) { ring = r.slice(0, -1); msg = ''; return; } // édition d'une zone fermée
      // sommets posés : compteur terra-draw (il compte aussi un sommet refusé par la validation) borné par l'anneau
      // en cours (sommets + curseur + fermeture ; 4 dès le 1er point)
      const f = drawing();
      points = f ? Math.min(Number(f.properties.provisionalCoordinateCount ?? 0), (r?.length ?? 2) - 2) : 0;
      msg = points >= MAX ? t().zone.tooMany : '';
    });
    draw.on('finish', (id, ctx) => {
      if (ctx.mode !== 'polygon' || ctx.action !== 'draw') return;
      const r = readSnapshot();
      if (r) { ring = r.slice(0, -1); points = ring.length; }
      edit(id);
    });
    return () => {
      draw?.stop();
      draw = null;
      setTimeout(() => (tmap.drawing = false), 0);
    };
  });

  function edit(id: string | number) {
    draw!.setMode('select');
    draw!.selectFeature(id);
  }

  function undo() {
    if (ring) return clear();
    draw?.undo();
  }
  function clear() {
    draw?.clear();
    draw?.setMode('polygon');
    ring = null;
    points = 0;
    msg = '';
  }
  async function validate() {
    if (!valid || !ring) return;
    app.zone = ring;
    await close();
  }
  async function removeZone() {
    app.zone = null;
    await close();
  }
</script>

<div class="head">
  <button class="icon-btn" onclick={close} aria-label={t().compute.cancel}>✕</button>
  <h2>{t().zone.button}</h2>
  <span class="count" aria-live="polite">{ring ? ring.length : points}/{MAX}</span>
</div>
<p>{ring ? t().zone.editHint : t().zone.hint}</p>
<p class="msg" role="alert">{msg}</p>
{#if outside}<p class="hint-out" data-testid="zone-outside">{t().zone.outside}</p>{/if}
<div class="actions">
  <button class="btn secondary" onclick={undo} disabled={!ring && points === 0}>{t().zone.undo}</button>
  <button class="btn secondary" onclick={clear}>{t().zone.clear}</button>
  <button class="btn primary" onclick={validate} disabled={!valid} data-testid="zone-validate">{t().zone.validate}</button>
</div>
{#if app.zone}<button class="btn link" onclick={removeZone}>{t().zone.remove}</button>{/if}

<style>
  .head { display: flex; gap: 8px; align-items: center; }
  h2 { font-size: 1.25rem; margin: 0; flex: 1; }
  .count { color: var(--muted); font-variant-numeric: tabular-nums; }
  .msg { color: var(--danger); min-height: 1.2em; margin: 4px 0; }
  .msg:empty { display: none; }
  .hint-out { color: var(--warn-text); margin: 4px 0; }
  .actions { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
</style>
