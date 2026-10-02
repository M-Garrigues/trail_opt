"""Boucle de trail à D+ optimal : interface Streamlit (affichage uniquement)."""
from __future__ import annotations

import base64
import math
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import folium
import numpy as np
import streamlit as st
import streamlit.components.v1 as components
from streamlit_folium import st_folium

from trailopt.geo import LocalFrame
from trailopt import geocode
from trailopt.maplayers import ProfileLink, ScaleControl, add_ign_layers
from trailopt.pipeline import (DIST_KM, MAX_AREA_KM2, TIME_S, Cancelled, Params, UserError, _rings,
                               build_region, plan_loop)

DEFAULT_CENTER = (48.7303, 2.2725)  # Massy (91), centre-ville
START_COLOR, ZONE_COLOR = "#1a9e3f", "#1f6feb"


def _cursor(svg: str, x: int, y: int) -> str:
    return f'url("data:image/svg+xml;base64,{base64.b64encode(svg.encode()).decode()}") {x} {y}'


# Curseur selon le mode de clic : épingle verte (départ) ou viseur + polygone (zone).
# Le mode est porté par un élément invisible des calques dynamiques (classe mode-*),
# lu en CSS par :has() : le fond de carte n'est jamais rechargé.
CURSOR_START = _cursor(
    "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>"
    f"<path d='M16 31C16 31 5 18 5 11a11 11 0 0 1 22 0c0 7-11 20-11 20z' fill='{START_COLOR}' "
    "stroke='white' stroke-width='2'/><circle cx='16' cy='11' r='4' fill='white'/></svg>", 16, 31)
CURSOR_ZONE = _cursor(
    "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'><g stroke-linecap='round'>"
    "<path d='M16 3v9M16 20v9M3 16h9M20 16h9' stroke='white' stroke-width='5'/>"
    f"<path d='M16 3v9M16 20v9M3 16h9M20 16h9' stroke='{ZONE_COLOR}' stroke-width='2.5'/></g>"
    f"<path d='M21 21l9 2-2 7-8-1z' fill='white' fill-opacity='.7' stroke='{ZONE_COLOR}' "
    "stroke-width='1.8' stroke-linejoin='round'/></svg>", 16, 16)
MAP_CSS = f"""<style>
.trail-profile {{ background: rgba(255,255,255,.93); border-radius: 6px; padding: 2px;
  box-shadow: 0 1px 5px rgba(0,0,0,.35); line-height: 0; }}
.leaflet-bottom .trail-profile {{ margin-bottom: 3px; }}
.leaflet-container:has(.trail-profile) .leaflet-bottom.leaflet-right {{ margin-bottom: 126px; }}
.trail-profile canvas {{ cursor: crosshair !important; }}
.hillshade {{ mix-blend-mode: multiply; filter: brightness(1.45) contrast(1.25); opacity: 0.75; }}
.leaflet-container:has(.mode-start), .leaflet-container:has(.mode-start) .leaflet-interactive
  {{ cursor: {CURSOR_START}, crosshair !important; }}
.leaflet-container:has(.mode-zone), .leaflet-container:has(.mode-zone) .leaflet-interactive
  {{ cursor: {CURSOR_ZONE}, crosshair !important; }}
</style>"""
STEPS = {"chemins": "Réseau de chemins…", "altitude": "Altitude IGN…",
         "élagage": "Élagage du graphe…", "solveur": "Optimisation…"}
ROADS = {"Sentiers (non revêtus)": "unpaved", "Voies piétonnes": "pedestrian",
         "+ petites routes": "minor", "+ toutes routes": "all"}

st.set_page_config(page_title="Boucle trail D+", page_icon="⛰️", layout="wide")
# Moins d'espace vide au-dessus du titre (6rem par défaut dans Streamlit).
st.markdown("<style>.block-container { padding-top: 2.2rem; }</style>", unsafe_allow_html=True)


@st.cache_resource
def compute_lock() -> threading.Lock:
    """Un seul calcul à la fois pour toute l'app (2 cœurs partagés)."""
    return threading.Lock()


ss = st.session_state
for k in ("start", "polygon", "result", "view"):
    ss.setdefault(k, None)


@st.cache_data(ttl=3600, show_spinner=False)
def geocode_cached(q: str) -> list:
    return geocode.search(q)

# ---------------------------------------------------------------- paramètres
with st.sidebar:
    st.header("Paramètres")
    distance = st.number_input("Distance (km)", DIST_KM[0], DIST_KM[1], 10.0, 0.5)
    mode_label = st.radio("Objectif", ["Maximiser le D+", "Cible distance × D+"])
    mode = "max" if mode_label.startswith("Max") else "target"
    target = None
    if mode == "target":
        target = st.number_input("D+ cible (m)", 10.0, 5000.0, 300.0, 10.0)
    node_simple = st.checkbox(
        "Ne jamais repasser par un carrefour", value=True,
        help="Hors d'un rayon de 200 m autour du départ. Dans ce rayon, on peut toujours "
             "reprendre la même route (accès en impasse).")
    with st.expander("Options avancées"):
        grade = st.number_input("Pente max (%) — 0 = aucune", 0.0, 100.0, 0.0, 1.0)
        time_s = st.slider("Temps de calcul max (s)", int(TIME_S[0]), int(TIME_S[1]), 20)
        tol = st.slider("Tolérance distance, mode max (± %)", 1, 20, 5)
        source = st.radio("Source des chemins", ["IGN (BD TOPO)", "OpenStreetMap"],
                          help="IGN : géométrie précise, ponts et tunnels fiables, mais pas de "
                               "trottoirs ni de petites voies piétonnes en ville. "
                               "OpenStreetMap : plus complet en ville et sur les sentiers informels.")
        roads = st.radio("Types de voies", list(ROADS), index=2,
                         help="Sentiers : chemins de terre, gravier, herbe… "
                              "Voies piétonnes : sentiers + trottoirs, voies piétonnes, pistes cyclables.")
        if ROADS[roads] == "pedestrian" and source.startswith("IGN"):
            st.caption("Voies piétonnes : OpenStreetMap est utilisé, car l'IGN ne décrit pas "
                       "les trottoirs ni les petites voies piétonnes.")
    st.caption(f"Zone ≤ {MAX_AREA_KM2:g} km² après découpe au rayon atteignable. "
               "Données © IGN, © contributeurs OpenStreetMap.")



def make_params() -> Params:
    """Toujours reconstruit depuis l'état courant (départ et zone peuvent changer en cours de run)."""
    return Params(
        lat=ss.start[0] if ss.start else 0.0, lon=ss.start[1] if ss.start else 0.0,
        distance_km=distance, polygon=ss.polygon if len(ss.polygon or []) >= 3 else None, mode=mode, target_dplus=target,
        max_grade=grade / 100 if grade > 0 else None, time_s=float(time_s), tol=tol / 100,
        roads=ROADS[roads], node_simple=node_simple,
        source="ign" if source.startswith("IGN") else "osm")


# ---------------------------------------------------------------- carte
t1, t2 = st.columns([5, 1], vertical_alignment="center")
t1.title("Boucle de trail à D+ optimal")
if (ss.start or ss.polygon) and t2.button("Tout effacer", type="secondary", use_container_width=True):
    ss.start = ss.polygon = ss.result = None
    st.rerun()
st.write("Clique sur la carte pour placer le **départ**, puis, si tu veux, les sommets de la "
         "**zone** : elle se ferme toute seule dès 3 points. Sans zone : disque centré sur le départ.")

# Recherche d'adresse, de commune, de lieu ou de coordonnées : recentre la carte.
s1, s2, s3 = st.columns([2.2, 2.6, 1.2], vertical_alignment="bottom")
query = s1.text_input("Chercher une adresse, une ville, un lieu ou « lat, lon »",
                      placeholder="ex. Chamonix, col du Galibier, 45.92, 6.87")
if query:
    found = geocode_cached(query)
    if not found:
        s2.caption("Aucun résultat.")
    else:
        pick = s2.selectbox("Résultats", range(len(found)), format_func=lambda i: found[i]["label"])
        hit = found[pick]
        if ss.get("last_search") != (query, pick):   # ne recentre qu'à une nouvelle recherche
            ss.last_search = (query, pick)
            ss.view = (hit["lat"], hit["lon"], hit["zoom"])
            ss.focus = "Départ ici"          # Entrée à nouveau : place le départ ici
        if s3.button("Départ ici", use_container_width=True):
            ss.start = (hit["lat"], hit["lon"])
            ss.focus = "Calculer"            # Entrée à nouveau : lance le calcul
            st.rerun()



def fit_view(lat, lon, width_px=900, height_px=410, profile_px=125):
    """Centre et zoom qui cadrent la boucle au-dessus du bandeau de profil."""
    la0, la1, lo0, lo1 = lat.min(), lat.max(), lon.min(), lon.max()
    mid = (la0 + la1) / 2
    h_m = max((la1 - la0) * 111_320, 50.0)
    w_m = max((lo1 - lo0) * 111_320 * math.cos(math.radians(mid)), 50.0)
    res = max(h_m / height_px, w_m / width_px) * 1.15            # m/pixel nécessaires
    zoom = int(min(18, max(3, math.floor(math.log2(156_543.03 * math.cos(math.radians(mid)) / res)))))
    res_z = 156_543.03 * math.cos(math.radians(mid)) / 2 ** zoom
    return (mid - (profile_px / 2) * res_z / 111_320, (lo0 + lo1) / 2, zoom)


# Calcul : bouton sous la recherche dès qu'un départ est posé, barre de progression dessous.
# Conteneur fixe : bouton, barre de progression et messages y apparaissent sans décaler la
# suite de la page (sinon le navigateur recharge le cadre de la carte).
with st.container():
    if ss.pop("cancelled", False):
        st.info("Calcul annulé.")
    if ss.start is None:
        st.info("Clique sur la carte pour placer le départ : le bouton Calculer apparaîtra ici.")
    elif st.button("Calculer", type="primary"):
        params = make_params()
        lock = compute_lock()
        if not lock.acquire(blocking=False):
            st.warning("Calcul en cours, réessaie dans quelques secondes.")
        else:
            try:
                state = {"step": "chemins"}
                bar = st.progress(0.0, text=STEPS["chemins"])
                # Annuler (ou Échap) relance le script : l'exception d'interruption de Streamlit
                # passe par le `except BaseException` ci-dessous, qui arrête vraiment le solveur.
                st.button("Annuler", help="ou touche Échap",
                          on_click=lambda: ss.update(cancelled=True))
                expected = params.time_s + 6.0          # budget solveur + chargement des données
                t0 = time.time()
                cancel = threading.Event()
                with ThreadPoolExecutor(1) as ex:
                    fut = ex.submit(plan_loop, params, lambda step: state.update(step=step), cancel)
                    try:
                        while not fut.done():
                            frac = min(0.97, (time.time() - t0) / expected)
                            bar.progress(frac, text=STEPS.get(state["step"], state["step"]))
                            time.sleep(0.2)
                    except BaseException:
                        cancel.set()        # le fil de calcul s'arrête, puis le verrou est rendu
                        raise
                    ss.result = fut.result()
                bar.progress(1.0, text="Terminé")
                ss.view = fit_view(ss.result.lat, ss.result.lon)
                ss.scroll_to_result = True
                st.rerun()
            except Cancelled:
                st.info("Calcul annulé.")
            except UserError as ex:
                st.error(str(ex))
            except Exception as ex:  # réseau, solveur...
                st.error(f"Échec du calcul : {ex}")
            finally:
                lock.release()

c1, c2, c3 = st.columns([3, 1.3, 1])
click_mode = c1.radio("Un clic sur la carte place", ["le départ", "un point de zone"],
                      index=0 if ss.start is None else 1, horizontal=True)
if c2.button("Annuler le dernier point", disabled=not ss.polygon):
    ss.polygon = ss.polygon[:-1] or None
    st.rerun()
if c3.button("Effacer la zone", disabled=not ss.polygon):
    ss.polygon = None
    st.rerun()

# Fond de carte statique (jamais rechargé : la vue et le zoom restent en place) ;
# tout ce qui bouge passe par `feature_group_to_add`.
m = folium.Map(location=DEFAULT_CENTER, zoom_start=13, tiles=None)
ScaleControl().add_to(m)
add_ign_layers(m)
m.get_root().header.add_child(folium.Element(MAP_CSS))
fg = folium.FeatureGroup(name="calques", control=False)
folium.CircleMarker(ss.start or DEFAULT_CENTER, radius=0, opacity=0, fill_opacity=0,
                    class_name="mode-start" if click_mode == "le départ" else "mode-zone").add_to(fg)
# Les clics sur les tracés remontent à la carte (bubblingMouseEvents, défaut Leaflet).

zone_error = None
pts = ss.polygon or []
if len(pts) >= 3:
    folium.Polygon([(la, lo) for lo, la in pts], color="#555", weight=1.5, fill=False).add_to(fg)
elif len(pts) == 2:
    folium.PolyLine([(la, lo) for lo, la in pts], color="#555", weight=1.5).add_to(fg)
for lo, la in pts:
    folium.CircleMarker((la, lo), radius=4, color="#555", fill=True, fill_opacity=1).add_to(fg)
if ss.start:
    folium.Marker(ss.start, tooltip="Départ",
                  icon=folium.Icon(color="green", icon="play")).add_to(fg)
    try:
        frame = LocalFrame(*ss.start)
        Lmax = distance * 1000 * ((1 + tol / 100) if mode == "max" else 1.3)
        for ring in _rings(build_region(make_params(), frame, Lmax), frame):
            folium.Polygon([(la, lo) for lo, la in ring], color=ZONE_COLOR, weight=2,
                           dash_array="6", fill=True, fill_opacity=0.06).add_to(fg)
    except UserError as ex:
        zone_error = str(ex)
res = ss.result
if res is not None:
    folium.PolyLine(list(zip(res.lat, res.lon)), color="#d62728", weight=4).add_to(fg)
    if (res.debug.get("start_snap_m") or 0) > 30:
        folium.CircleMarker((res.lat[0], res.lon[0]), radius=7, color="#d62728", fill=True).add_to(fg)
    keep = np.unique(np.linspace(0, len(res.dist) - 1, 800).astype(int))
    profile = {"d": np.round(res.dist[keep] / 1000, 3).tolist(), "z": np.round(res.ele[keep], 1).tolist(),
               "lat": np.round(res.lat[keep], 6).tolist(), "lon": np.round(res.lon[keep], 6).tolist()}
else:
    profile = None
ProfileLink(profile).add_to(fg)  # toujours présent : sans résultat, il retire l'ancien profil

out = st_folium(m, height=620, use_container_width=True, key="map", feature_group_to_add=fg,
                center=ss.view[:2] if ss.view else None, zoom=ss.view[2] if ss.view else None,
                returned_objects=["last_clicked"])
if res is not None:  # résultat juste sous la carte, GPX à droite
    n = 5 if "err_dplus" in res.debug else 3
    c = st.columns([1] * n + [1.3], vertical_alignment="center")
    c[0].metric("Distance", f"{res.length / 1000:.2f} km")
    c[1].metric("D+", f"{res.dplus:.0f} m")
    c[2].metric("D+/km", f"{res.dplus / res.length * 1000:.0f} m")
    if n == 5:
        c[3].metric("Écart distance", f"{res.debug['err_distance']:+.1%}")
        c[4].metric("Écart D+", f"{res.debug['err_dplus']:+.1%}")
    c[-1].download_button("Télécharger le GPX", res.gpx, file_name=f"boucle_{distance:g}km.gpx",
                          mime="application/gpx+xml", type="primary", use_container_width=True)
    for w in res.warnings:
        st.warning(w)
st.caption("Vert : départ. Pointillés bleus : zone utile (zone dessinée coupée au rayon atteignable). "
           "Gris : sommets dessinés. Rouge : boucle ; rond rouge : départ effectif s'il a été déplacé. "
           "Fonds et relief : bouton de couches en haut à droite de la carte. "
           "Survole le profil ou le tracé : le point correspondant s'affiche sur l'autre.")
st.markdown('<div id="legend-end"></div>', unsafe_allow_html=True)
pending_js = []
if ss.pop("scroll_to_result", False):
    # Après un calcul : la légende juste au-dessus du bas de l'écran si carte + résultats
    # tiennent dans la fenêtre ; sinon on cale la page sur la carte.
    pending_js.append("""
    setTimeout(function () {
      const vh = window.parent.innerHeight, header = 60;  // barre d'outils Streamlit
      const map = d.querySelector('iframe[title="streamlit_folium.st_folium"]');
      const end = d.getElementById('legend-end');
      if (!map || !end) return;
      const span = end.getBoundingClientRect().bottom - map.getBoundingClientRect().top;
      if (span <= vh - header) {
        end.style.scrollMarginBottom = '8px';
        end.scrollIntoView({block: 'end', behavior: 'smooth'});
      } else {
        map.style.scrollMarginTop = header + 'px';
        map.scrollIntoView({block: map.getBoundingClientRect().height <= vh - header ? 'start' : 'center',
                            behavior: 'smooth'});
      }
    }, 400);""")
click = (out or {}).get("last_clicked")
if click and click != ss.get("last_click"):
    ss.last_click = click
    pt = (click["lat"], click["lng"])
    if click_mode == "le départ":
        ss.start = pt
        ss.focus = "Calculer"
    else:
        ss.polygon = (ss.polygon or []) + [(pt[1], pt[0])]
    st.rerun()   # le tracé affiché reste en place jusqu'au prochain « Calculer »
if zone_error:
    st.error(zone_error)

# ---------------------------------------------------------------- résultat
if res is not None:
    d = res.debug
    with st.expander("Debug", expanded=True):
        t = d.get("timings_s", {})
        lines = [
            f"**Solveur** : {d.get('solver')} — {d.get('solver_reason')} — {res.method}",
            f"**Graphe** ({d.get('source')}) : {d.get('source_ways')} voies → {d.get('raw_edges')} tronçons → "
            f"{d.get('edges_in_zone')} dans la zone → {d.get('edges_simplified')} simplifiés → "
            f"{d.get('edges_pruned')} après ponts/portée → {d.get('edges_after_grade')} après pente ; "
            f"{d.get('nodes')} nœuds ; {d.get('edges_doubled_near_start')} arêtes doublées "
            f"à moins de 200 m du départ, {d.get('edges_used_twice_near_start')} parcourues deux fois ; carrefours uniques : {'oui' if d.get('node_simple') else 'non'} ; "
            f"{d.get('parallel_pairs')} couloirs parallèles exclus",
            f"**Temps** : chargement des chemins {t.get('network_fetch_s')} s, altitude {t.get('elevation_s')} s, "
            f"préparation {t.get('prep_s')} s, solveur {t.get('solver_s')} s",
            f"**Mémoire** : pic RSS {d.get('peak_rss_mb')} Mo ; "
            f"{d.get('elevation_points')} points d'altitude ; zone {d.get('zone_km2')} km²",
            f"**Réseau** : {d.get('network')}",
        ]
        if d.get("cpsat_status"):
            gap = d.get("cpsat_gap")
            lines.append(f"**CP-SAT** : {d['cpsat_status']}, borne {d.get('cpsat_bound')}, "
                         f"écart {gap:.2%}" if gap is not None else "")
        if d.get("anneal_iterations") is not None:
            lines.append(f"**Recuit** : {d['anneal_iterations']} itérations")
        st.markdown("  \n".join(x for x in lines if x))
        st.json(d, expanded=False)

# ---------------------------------------------------------------- clavier
# Entrée enchaîne recherche -> « Départ ici » -> « Calculer » (focus posé sur le bon bouton) ;
# Échap clique « Annuler » pendant un calcul.
focus = ss.pop("focus", None)
if focus:
    pending_js.append("""
    setTimeout(function () {
      const b = Array.from(d.querySelectorAll('button')).find(x => x.innerText.trim() === %r);
      if (b) b.focus();
    }, 350);""" % focus)
page_js = """<script>
const d = window.parent.document;
if (!window.parent.__trailEscPage) {
  // Installé dans la page elle-même (pas dans ce cadre, recréé à chaque rafraîchissement).
  window.parent.__trailEscPage = true;
  const sc = d.createElement('script');
  sc.textContent = `document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    const b = Array.from(document.querySelectorAll('button')).find(x => x.innerText.trim() === 'Annuler');
    if (b) b.click();
  }, true);`;
  d.head.appendChild(sc);
}
%s
</script>""" % (("// %s" % time.time() + "".join(pending_js)) if pending_js else "")
if hasattr(st, "iframe"):       # Streamlit récent ; components.html est obsolète
    st.iframe(page_js, height=1)
else:
    components.html(page_js, height=0)
