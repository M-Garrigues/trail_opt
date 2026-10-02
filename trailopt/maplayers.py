"""Fonds de carte IGN pour folium : Plan IGN vectoriel (MapLibre GL), raster, photos, relief."""
from __future__ import annotations

import folium
from branca.element import MacroElement
from folium.elements import JSCSSMixin
from folium.map import Layer
from jinja2 import Template

WMTS = ("https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0"
        "&LAYER={layer}&STYLE=normal&TILEMATRIXSET=PM&FORMAT={fmt}"
        "&TILEMATRIX={{z}}&TILEROW={{y}}&TILECOL={{x}}")
PLAN_IGN_STYLE = ("https://data.geopf.fr/annexes/ressources/vectorTiles/styles/"
                  "PLAN.IGN/standard.json")


class MapLibreLayer(JSCSSMixin, Layer):
    """Fond vectoriel rendu par MapLibre GL dans Leaflet (plugin maplibre-gl-leaflet)."""
    _template = Template("""
        {% macro script(this, kwargs) %}
            var {{ this.get_name() }} = L.maplibreGL({
                style: {{ this.style|tojson }},
                attribution: {{ this.attr|tojson }},
                maxZoom: 19
            });
        {% endmacro %}
    """)
    default_js = [
        ("maplibre_js", "https://cdn.jsdelivr.net/npm/maplibre-gl@4.7.1/dist/maplibre-gl.js"),
        ("maplibre_leaflet",
         "https://cdn.jsdelivr.net/npm/@maplibre/maplibre-gl-leaflet@0.0.22/leaflet-maplibre-gl.js"),
    ]
    default_css = [
        ("maplibre_css", "https://cdn.jsdelivr.net/npm/maplibre-gl@4.7.1/dist/maplibre-gl.css"),
    ]

    def __init__(self, style: str, name: str, attr: str = "© IGN", show: bool = True):
        super().__init__(name=name, overlay=False, control=True, show=show)
        self._name = "MapLibreLayer"
        self.style, self.attr = style, attr


def add_ign_layers(m: folium.Map) -> None:
    """Fonds (vectoriel par défaut, raster, photos) + relief LiDAR HD en surimpression."""
    MapLibreLayer(PLAN_IGN_STYLE, "Plan IGN (vectoriel)").add_to(m)
    folium.TileLayer(WMTS.format(layer="GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2", fmt="image/png"),
                     attr="© IGN", name="Plan IGN (raster)", max_zoom=19, show=False).add_to(m)
    folium.TileLayer(WMTS.format(layer="ORTHOIMAGERY.ORTHOPHOTOS", fmt="image/jpeg"),
                     attr="© IGN", name="Photos aériennes", max_zoom=19, show=False).add_to(m)
    folium.TileLayer(
        WMTS.format(layer="IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.SHADOW", fmt="image/png"),
        attr="© IGN", name="Relief (ombrage LiDAR HD)", overlay=True, max_zoom=19,
        max_native_zoom=18, class_name="hillshade").add_to(m)
    folium.LayerControl(position="topright", collapsed=True).add_to(m)


class ProfileLink(MacroElement):
    """Profil altimétrique dessiné dans la carte et lié au tracé : survoler le profil
    montre le point sur la carte, survoler le tracé montre le point sur le profil.
    Tout est côté navigateur (aucun aller-retour serveur). `data=None` retire le profil.
    data = {d: km, z: m, lat, lon} (listes de même longueur)."""
    _template = Template("""
        {% macro script(this, kwargs) %}
        (function () {
          // Échap pendant un calcul, quand le focus est dans la carte : clique « Annuler »
          // dans la page (les touches frappées ici n'atteignent pas la page parente).
          if (!window.__trailEscMap) {
            window.__trailEscMap = true;
            document.addEventListener('keydown', function (e) {
              if (e.key !== 'Escape') return;
              try {
                var bs = window.parent.document.querySelectorAll('button');
                for (var i = 0; i < bs.length; i++)
                  if (bs[i].innerText.trim() === 'Annuler') { bs[i].click(); break; }
              } catch (err) {}
            }, true);
          }
          var map = window.map, old = window.__trailProfile;
          if (old) {
            try { map.removeControl(old.ctl); map.removeLayer(old.dot); map.removeLayer(old.hit);
                  map.off('resize', old.onResize); } catch (e) {}
            window.__trailProfile = null;
          }
          var D = {{ this.data|tojson }};
          if (!D || !map) return;
          var n = D.d.length, H = 117, PAD = {l: 46, r: 12, t: 10, b: 22}, dmax = D.d[n - 1];
          var zmin = Math.min.apply(null, D.z), zmax = Math.max.apply(null, D.z);
          if (zmax - zmin < 10) zmax = zmin + 10;
          // Pente (%) en chaque point, mesurée sur ~50 m pour gommer le bruit du MNT.
          var G = [], i0 = 0, i1 = 0;
          for (var gi = 0; gi < n; gi++) {
            while (i0 < gi && D.d[gi] - D.d[i0] > 0.025) i0++;
            while (i1 < n - 1 && D.d[i1] - D.d[gi] < 0.025) i1++;
            var run = (D.d[i1] - D.d[i0]) * 1000;
            G.push(run > 1 ? 100 * (D.z[i1] - D.z[i0]) / run : 0);
          }
          // Dégradé continu selon la pente absolue : vert à plat, rouge sombre à 40 % et plus.
          var STOPS = [[0, 76, 175, 80], [5, 205, 220, 57], [10, 255, 193, 7],
                       [18, 244, 81, 30], [28, 198, 40, 40], [40, 93, 15, 15]];
          function slopeColor(g) {
            g = Math.min(Math.abs(g), 40);
            for (var k = 1; k < STOPS.length; k++) {
              if (g <= STOPS[k][0]) {
                var a = STOPS[k - 1], b = STOPS[k], t = (g - a[0]) / (b[0] - a[0]);
                return 'rgb(' + Math.round(a[1] + t * (b[1] - a[1])) + ',' +
                       Math.round(a[2] + t * (b[2] - a[2])) + ',' + Math.round(a[3] + t * (b[3] - a[3])) + ')';
              }
            }
          }
          var dot = L.circleMarker([D.lat[0], D.lon[0]], {radius: 7, color: '#fff', weight: 2,
            fillColor: '#d62728', opacity: 0, fillOpacity: 0, interactive: false}).addTo(map);
          var hit = L.polyline(D.lat.map(function (la, i) { return [la, D.lon[i]]; }),
            {opacity: 0, weight: 24}).addTo(map);
          var ctl = L.control({position: 'bottomleft'}), canvas, cur = -1;
          function W() { return Math.max(260, map.getSize().x - 20); }
          function X(d, w) { return PAD.l + (w - PAD.l - PAD.r) * d / dmax; }
          function Y(z) { return PAD.t + (H - PAD.t - PAD.b) * (1 - (z - zmin) / (zmax - zmin)); }
          function draw() {
            var w = W(), r = window.devicePixelRatio || 1, i;
            canvas.width = w * r; canvas.height = H * r;
            canvas.style.width = w + 'px'; canvas.style.height = H + 'px';
            var c = canvas.getContext('2d');
            c.setTransform(r, 0, 0, r, 0, 0); c.clearRect(0, 0, w, H);
            c.font = '11px sans-serif'; c.lineWidth = 1;
            for (i = 0; i <= 2; i++) {
              var z = zmin + (zmax - zmin) * i / 2, y = Y(z);
              c.strokeStyle = '#ddd'; c.beginPath(); c.moveTo(PAD.l, y); c.lineTo(w - PAD.r, y); c.stroke();
              c.fillStyle = '#555'; c.textAlign = 'right'; c.fillText(Math.round(z) + ' m', PAD.l - 5, y + 4);
            }
            c.textAlign = 'center';
            var step = dmax > 12 ? 5 : (dmax > 5 ? 2 : 1);
            for (var km = 0; km <= dmax; km += step) c.fillText(km + ' km', X(km, w), H - 6);
            var base = H - PAD.b;
            c.globalAlpha = 0.8;
            for (i = 0; i < n - 1; i++) {   // remplissage coloré par la pente
              var xa = X(D.d[i], w), xb = X(D.d[i + 1], w);
              c.fillStyle = slopeColor((G[i] + G[i + 1]) / 2);
              c.beginPath(); c.moveTo(xa, base); c.lineTo(xa, Y(D.z[i]));
              c.lineTo(xb + 0.6, Y(D.z[i + 1])); c.lineTo(xb + 0.6, base); c.closePath(); c.fill();
            }
            c.globalAlpha = 1;
            c.beginPath(); c.moveTo(X(D.d[0], w), Y(D.z[0]));
            for (i = 1; i < n; i++) c.lineTo(X(D.d[i], w), Y(D.z[i]));
            c.strokeStyle = '#37474f'; c.lineWidth = 1.5; c.stroke();
            if (cur >= 0) {
              var x = X(D.d[cur], w), left = x > w / 2;
              c.strokeStyle = '#333'; c.lineWidth = 1;
              c.beginPath(); c.moveTo(x, PAD.t); c.lineTo(x, H - PAD.b); c.stroke();
              c.fillStyle = '#111'; c.beginPath(); c.arc(x, Y(D.z[cur]), 4, 0, 6.3); c.fill();
              c.fillStyle = '#111'; c.font = 'bold 12px sans-serif'; c.textAlign = left ? 'right' : 'left';
              var g = Math.round(G[cur]), txt = D.d[cur].toFixed(2) + ' km · ' + Math.round(D.z[cur])
                + ' m · ' + (g > 0 ? '+' : '') + g + ' %';
              var tw = c.measureText(txt).width + 8, tx = left ? x - 8 - tw : x + 8;
              c.fillStyle = 'rgba(255,255,255,.85)'; c.fillRect(tx, PAD.t - 1, tw, 16);
              c.fillStyle = '#111'; c.textAlign = 'left'; c.fillText(txt, tx + 4, PAD.t + 11);
            }
          }
          function show(i) {
            cur = i;
            if (i < 0) dot.setStyle({opacity: 0, fillOpacity: 0});
            else { dot.setLatLng([D.lat[i], D.lon[i]]); dot.setStyle({opacity: 1, fillOpacity: 1}); }
            draw();
          }
          ctl.onAdd = function () {
            var div = L.DomUtil.create('div', 'trail-profile');
            canvas = L.DomUtil.create('canvas', '', div);
            L.DomEvent.disableClickPropagation(div); L.DomEvent.disableScrollPropagation(div);
            canvas.addEventListener('mousemove', function (e) {
              var b = canvas.getBoundingClientRect(), lo = 0, hi = n - 1;
              var d = (e.clientX - b.left - PAD.l) / (W() - PAD.l - PAD.r) * dmax;
              while (hi - lo > 1) { var m = (lo + hi) >> 1; if (D.d[m] < d) lo = m; else hi = m; }
              show(Math.abs(D.d[lo] - d) < Math.abs(D.d[hi] - d) ? lo : hi);
            });
            canvas.addEventListener('mouseleave', function () { show(-1); });
            canvas.addEventListener('click', function () {   // centre la carte sur le point
              if (cur >= 0) map.panTo([D.lat[cur], D.lon[cur]]);
            });
            return div;
          };
          ctl.addTo(map); draw();
          hit.on('mousemove', function (e) {
            var best = 0, bd = Infinity, k = Math.cos(e.latlng.lat * Math.PI / 180);
            for (var i = 0; i < n; i++) {
              var dy = D.lat[i] - e.latlng.lat, dx = (D.lon[i] - e.latlng.lng) * k, q = dx * dx + dy * dy;
              if (q < bd) { bd = q; best = i; }
            }
            show(best);
          });
          hit.on('mouseout', function () { show(-1); });
          // Les calques de la boucle sont ajoutés après ce script : repasser au-dessus.
          setTimeout(function () { if (window.__trailProfile && window.__trailProfile.hit === hit) hit.bringToFront(); }, 50);
          var onResize = function () { draw(); };
          map.on('resize', onResize);
          window.__trailProfile = {ctl: ctl, dot: dot, hit: hit, onResize: onResize};
        })();
        {% endmacro %}
    """)

    def __init__(self, data: dict | None):
        super().__init__()
        self._name = "ProfileLink"
        self.data = data


class ScaleControl(MacroElement):
    """Échelle en bas à droite : le bas gauche est réservé au profil."""
    _template = Template("""
        {% macro script(this, kwargs) %}
            L.control.scale({position: 'bottomright', imperial: false}).addTo({{ this._parent.get_name() }});
        {% endmacro %}
    """)
