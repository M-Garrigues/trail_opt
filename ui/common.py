"""Briques d'interface Streamlit partagées par les vues bureau (desktop) et mobile (mobile).

Aucune logique de calcul ici : tout passe par trailopt.pipeline.
"""
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

from trailopt import geocode
from trailopt.geo import LocalFrame
from trailopt.maplayers import ProfileLink, ScaleControl, add_ign_layers
from trailopt.pipeline import (DIST_KM, LONG_KM, TIME_S, Cancelled, Params, UserError, _rings,
                               build_region, plan_loop, suggested_time)

DEFAULT_CENTER = (48.7303, 2.2725)  # Massy (91), centre-ville
START_COLOR, ZONE_COLOR = "#1a9e3f", "#1f6feb"
# Palette tab20 (matplotlib), teintes foncées d'abord : plus lisibles sur une carte.
TAB20 = ["#1f77b4", "#aec7e8", "#ff7f0e", "#ffbb78", "#2ca02c", "#98df8a", "#d62728", "#ff9896",
         "#9467bd", "#c5b0d5", "#8c564b", "#c49c94", "#e377c2", "#f7b6d2", "#7f7f7f", "#c7c7c7",
         "#bcbd22", "#dbdb8d", "#17becf", "#9edae5"]
LOOP_COLORS = TAB20[0::2] + TAB20[1::2]
SINGLE_COLOR = "#d62728"       # une seule boucle : rouge, comme avant


def loop_color(res, i: int) -> str:
    return SINGLE_COLOR if len(res.candidates) < 2 else LOOP_COLORS[i % len(LOOP_COLORS)]
SEARCH_LABEL = "Chercher une adresse, une ville, un lieu ou « lat, lon »"
CLICK_MODES = ["le départ", "un point de zone"]


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
.trail-profile canvas {{ cursor: crosshair !important; touch-action: none; }}
.trail-summary {{ display: none; font: 700 15px sans-serif; color: #111; line-height: 1.2;
  padding: 7px 8px 3px 8px; }}
.trail-full .trail-summary {{ display: block; }}
.trail-full .leaflet-container:has(.trail-summary) .leaflet-bottom.leaflet-right {{ margin-bottom: 154px; }}
.leaflet-bar a.trail-fs {{ width: 44px; height: 44px; line-height: 44px; font-size: 22px;
  text-align: center; color: #333; }}
.trail-full, .trail-full body, .trail-full #parent, .trail-full #map_div, .trail-full #map_div2,
.trail-full .folium-map {{ height: 100vh !important; }}
.hillshade {{ mix-blend-mode: multiply; filter: brightness(1.45) contrast(1.25); opacity: 0.75; }}
.leaflet-container:has(.mode-start), .leaflet-container:has(.mode-start) .leaflet-interactive
  {{ cursor: {CURSOR_START}, crosshair !important; }}
.leaflet-container:has(.mode-zone), .leaflet-container:has(.mode-zone) .leaflet-interactive
  {{ cursor: {CURSOR_ZONE}, crosshair !important; }}
/* Plusieurs boucles proposées : la carte ne pose plus de point, seuls les tracés se cliquent. */
.leaflet-container:has(.mode-view), .leaflet-container:has(.mode-view) .leaflet-interactive
  {{ cursor: default !important; }}
.leaflet-container:has(.mode-view) path.trail-cand {{ cursor: pointer !important; }}
path.trail-active {{ animation: trail-march 1.5s linear infinite; pointer-events: none; }}
@keyframes trail-march {{ to {{ stroke-dashoffset: -24; }} }}
</style>"""
STEPS = {"chemins": "Réseau de chemins…", "criblage": "Repérage du relief…", "altitude": "Altitude IGN…",
         "élagage": "Élagage du graphe…", "solveur": "Optimisation…"}
ROADS = {"Sentiers (non revêtus)": "unpaved", "Voies piétonnes": "pedestrian",
         "+ petites routes": "minor", "+ toutes routes": "all"}

ss = st.session_state


def init_state() -> None:
    for k in ("start", "polygon", "result", "view"):
        ss.setdefault(k, None)


@st.cache_resource
def compute_lock() -> threading.Lock:
    """Un seul calcul à la fois pour toute l'app (2 cœurs partagés)."""
    return threading.Lock()


@st.cache_data(ttl=3600, show_spinner=False)
def geocode_cached(q: str) -> list:
    return geocode.search(q)


# ---------------------------------------------------------------- réglages
def params_form(compact: bool = False) -> dict:
    """Widgets de réglage. Bureau : dans la barre latérale. Mobile (compact) : une ligne,
    la distance à gauche et le reste dans un panneau flottant « Réglages »."""
    if compact:
        with st.container(key="row-params"):
            a, b = st.columns(2, vertical_alignment="bottom")
        v = {"distance": a.number_input("Distance (km)", DIST_KM[0], DIST_KM[1], 10.0, 0.5)}
        rest = b.popover("⚙️ Réglages", use_container_width=True)
    else:
        v = {"distance": st.number_input("Distance (km)", DIST_KM[0], DIST_KM[1], 10.0, 0.5)}
        rest = st.container()
    with rest:
        mode_label = st.radio("Objectif", ["Maximiser le D+", "Cible distance × D+"])
        v["mode"] = "max" if mode_label.startswith("Max") else "target"
        v["target"] = (st.number_input("D+ cible (m)", 10.0, 5000.0, 300.0, 10.0)
                       if v["mode"] == "target" else None)
        v["node_simple"] = st.checkbox(
            "Ne jamais repasser par un carrefour", value=True,
            help="Hors d'un rayon de 200 m autour du départ. Dans ce rayon, on peut toujours "
                 "reprendre la même route (accès en impasse).")
        with (st.container() if compact else st.expander("Options avancées")):
            v["grade"] = st.number_input("Pente max (%) — 0 = aucune", 0.0, 100.0, 0.0, 1.0)
            v["n_candidates"] = st.number_input(
                "Nombre de boucles proposées", 1, 4, 1,
                help="Boucles réellement différentes (moins de la moitié en commun). Chaque "
                     "boucle en plus ajoute la moitié du temps de calcul.")
            # Valeur conseillée selon la distance et le nombre de boucles ; le curseur y revient quand la distance change.
            v["time_s"] = st.slider("Temps de calcul (s)", int(TIME_S[0]), int(TIME_S[1]),
                                    int(suggested_time(v["distance"], v["n_candidates"])), step=5,
                                    help="Ajusté à la distance (20 s jusqu'à 10 km, 60 s à 100 km) "
                                         "et au nombre de boucles (moitié en plus par boucle). "
                                         "Au-delà de 60 s, le gain de D+ mesuré est d'environ 1 %.")
            v["tol"] = st.slider("Tolérance distance, mode max (± %)", 1, 20, 5)
            source = st.radio("Source des chemins", ["IGN (BD TOPO)", "OpenStreetMap"],
                              help="IGN : géométrie précise, ponts et tunnels fiables, mais pas de "
                                   "trottoirs ni de petites voies piétonnes en ville. "
                                   "OpenStreetMap : plus complet en ville et sur les sentiers informels.")
            roads = st.radio("Types de voies", list(ROADS), index=2,
                             help="Sentiers : chemins de terre, gravier, herbe… "
                                  "Voies piétonnes : sentiers + trottoirs, voies piétonnes, pistes cyclables.")
            v["source"] = "ign" if source.startswith("IGN") else "osm"
            v["roads"] = ROADS[roads]
            if v["roads"] == "pedestrian" and v["source"] == "ign":
                st.caption("Voies piétonnes : OpenStreetMap est utilisé, car l'IGN ne décrit pas "
                           "les trottoirs ni les petites voies piétonnes.")
        st.caption("Zone : rayon de 25 km au plus, réduite automatiquement si elle contient trop "
                   "de voies. Données © IGN, © contributeurs OpenStreetMap.")
    if v["distance"] > LONG_KM:
        st.caption(f"Plus de {LONG_KM:g} km : chargement des données et calcul plus longs "
                   f"(environ {suggested_time(v['distance'], v['n_candidates']):.0f} s de calcul).")
    return v


def make_params(v: dict) -> Params:
    """Toujours reconstruit depuis l'état courant (départ et zone peuvent changer en cours de run)."""
    return Params(
        lat=ss.start[0] if ss.start else 0.0, lon=ss.start[1] if ss.start else 0.0,
        distance_km=v["distance"], polygon=ss.polygon if len(ss.polygon or []) >= 3 else None,
        mode=v["mode"], target_dplus=v["target"],
        max_grade=v["grade"] / 100 if v["grade"] > 0 else None,
        time_s=float(v["time_s"]),
        tol=v["tol"] / 100, roads=v["roads"], node_simple=v["node_simple"], source=v["source"],
        n_candidates=int(v["n_candidates"]))


def selected(res) -> int:
    """Indice de la boucle mise en avant (la première, au plus grand D+, par défaut)."""
    return min(int(ss.get("cand", 0)), len(res.candidates) - 1)


def locked() -> bool:
    """Plusieurs boucles proposées : les clics sur la carte ne servent qu'à en choisir une
    (« Tout effacer » ou un nouveau calcul pour replacer départ et zone)."""
    return ss.result is not None and len(ss.result.candidates) > 1


def shown_loop(res):
    return res.candidates[selected(res)]


def candidate_list(res, v: dict, compact: bool = False) -> None:
    """Une ligne par boucle proposée : un clic sur la ligne (couleur, chiffres) l'affiche,
    le bouton GPX la télécharge. La ligne de la boucle affichée est mise en avant."""
    sel = selected(res)
    css = []
    for i, c in enumerate(res.candidates):
        color = loop_color(res, i)
        on = i == sel
        css.append(
            f".st-key-show{i} button {{ justify-content:flex-start; text-align:left; "
            + (f"border:2px solid {color}; background:{color}1f; font-weight:700;" if on else "")
            + f" }} .st-key-show{i} button p::before {{ content:''; display:inline-block; width:14px; "
            f"height:14px; border-radius:50%; background:{color}; margin-right:.55rem; vertical-align:-2px; }}"
            + (f" .st-key-show{i} button p {{ font-weight:700; }}" if on else ""))
        text = (f"Boucle {i + 1} — {c.length / 1000:.2f} km · D+ {c.dplus:.0f} m · "
                f"{c.dplus / c.length * 1000:.0f} m/km")
        with st.container(key=f"row-cand-{i}"):
            a, d = st.columns([4.3, 1.2] if not compact else [4.2, 1], vertical_alignment="center")
            if a.button(text, key=f"show{i}", use_container_width=True) and not on:
                ss.cand = i
                st.rerun()
            d.download_button("GPX", c.gpx, file_name=f"boucle_{v['distance']:g}km_{i + 1}.gpx",
                              mime="application/gpx+xml", key=f"gpx{i}", use_container_width=True,
                              type="primary" if on else "secondary")
    st.markdown(f"<style>{' '.join(css)}</style>", unsafe_allow_html=True)


# ---------------------------------------------------------------- recherche
def search_box(compact: bool = False) -> None:
    """Recherche d'adresse, de commune, de lieu ou de coordonnées : recentre la carte.
    Mobile (compact) : champs empilés, et bouton « Ma position » (géolocalisation)."""
    if compact:
        s2 = s3 = st
        with st.container(key="row-search"):     # champ + bouton position sur une ligne
            a, b = st.columns([5, 1])
            query = a.text_input(SEARCH_LABEL, placeholder="Adresse, ville, lieu ou « lat, lon »",
                                 label_visibility="collapsed")
            if b.button("📍", help="Partir de ma position", use_container_width=True):
                ss.geo_until = time.time() + 30  # les coordonnées reçues poseront le départ
                ss.want_geo = True
    else:
        s1, s2, s3 = st.columns([2.2, 2.6, 1.2], vertical_alignment="bottom")
        query = s1.text_input(SEARCH_LABEL, placeholder="ex. Chamonix, col du Galibier, 45.92, 6.87")
    if query:
        found = geocode_cached(query)
        if not found:
            s2.caption("Aucun résultat.")
        else:
            pick = s2.selectbox("Résultats", range(len(found)), format_func=lambda i: found[i]["label"],
                                label_visibility="collapsed" if compact else "visible")
            hit = found[pick]
            geo = ss.get("geo_until", 0) > time.time() and len(found) == 1   # coordonnées GPS reçues
            if ss.get("last_search") != (query, pick):   # ne recentre qu'à une nouvelle recherche
                ss.last_search = (query, pick)
                ss.view = (hit["lat"], hit["lon"], hit["zoom"])
                ss.focus = "Départ ici"          # Entrée à nouveau : place le départ ici
                if geo:
                    ss.geo_until = 0
                    ss.start = (hit["lat"], hit["lon"])
                    ss.focus = "Calculer"
                    st.rerun()
            if s3.button("Départ ici", use_container_width=True):
                ss.start = (hit["lat"], hit["lon"])
                ss.focus = "Calculer"            # Entrée à nouveau : lance le calcul
                st.rerun()


GEO_JS = """
(function () {
  const nav = window.parent.navigator;
  if (!nav.geolocation) { window.parent.alert('Géolocalisation indisponible sur cet appareil.'); return; }
  nav.geolocation.getCurrentPosition(function (p) {
    // Les coordonnées passent par le champ de recherche, qui sait lire « lat, lon ».
    const inp = d.querySelector('input[aria-label="%s"]');
    if (!inp) return;
    const set = Object.getOwnPropertyDescriptor(window.parent.HTMLInputElement.prototype, 'value').set;
    inp.focus();
    set.call(inp, p.coords.latitude.toFixed(5) + ', ' + p.coords.longitude.toFixed(5));
    inp.dispatchEvent(new window.parent.Event('input', {bubbles: true}));
    inp.dispatchEvent(new window.parent.KeyboardEvent('keydown',
      {key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true}));
    inp.blur();
  }, function (e) {
    window.parent.alert('Position indisponible : ' + e.message);
  }, {enableHighAccuracy: true, timeout: 15000, maximumAge: 60000});
})();""" % SEARCH_LABEL


# ---------------------------------------------------------------- calcul
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


def calc_block(v: dict, fit: dict | None = None, full_width: bool = False,
               cancel_label: str = "Annuler") -> None:
    """Bouton Calculer (grisé tant qu'aucun départ n'est posé), barre de progression, Annuler, messages.
    À appeler dans un conteneur fixe : rien ne doit décaler la suite de la page, sinon le
    navigateur recharge le cadre de la carte."""
    if ss.pop("cancelled", False):
        st.info("Calcul annulé.")
    if ss.start is None:
        if full_width:
            st.caption("Touche la carte pour placer le départ.")
        else:
            # Bouton déjà là, grisé : poser le départ ne décale pas la carte sous le curseur.
            st.button("Calculer", type="primary", disabled=True, key="btn-calc-off",
                      help="Clique d'abord sur la carte pour placer le départ.")
        return
    if not st.button("Calculer", type="primary", use_container_width=full_width, key="btn-calc"):
        return
    params = make_params(v)
    lock = compute_lock()
    if not lock.acquire(blocking=False):
        st.warning("Calcul en cours, réessaie dans quelques secondes.")
        return
    try:
        state = {"step": "chemins"}
        bar = st.progress(0.0, text=STEPS["chemins"])
        # Annuler (ou Échap) relance le script : l'exception d'interruption de Streamlit
        # passe par le `except BaseException` ci-dessous, qui arrête vraiment le solveur.
        st.button(cancel_label, help="Annuler (ou touche Échap)", key="btn-cancel",
                  on_click=lambda: ss.update(cancelled=True))
        # budget solveur + chargement des données (bien plus long sur les grandes zones)
        expected = params.time_s + 6.0 + 0.4 * max(0.0, params.distance_km - 10.0)
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
        ss.cand = 0                             # nouveau calcul : boucle au plus grand D+
        bar.progress(1.0, text="Terminé")
        ss.view = fit_view(ss.result.lat, ss.result.lon, **(fit or {}))
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


# ---------------------------------------------------------------- carte
def render_map(v: dict, click_mode: str, height: int, touch: bool = False, extra_css: str = ""):
    """Carte + calques dynamiques + profil lié. Renvoie (retour st_folium, erreur de zone).
    Le fond est statique (jamais rechargé : vue et zoom restent en place) ; tout ce qui
    bouge passe par `feature_group_to_add`."""
    # keyboard=False : sinon Leaflet prend le focus au premier appui, la page défile pour
    # montrer toute la carte, et le clic se pose décalé d'autant.
    m = folium.Map(location=DEFAULT_CENTER, zoom_start=13, tiles=None, keyboard=False)
    ScaleControl().add_to(m)
    add_ign_layers(m)
    m.get_root().header.add_child(folium.Element(MAP_CSS + extra_css))
    fg = folium.FeatureGroup(name="calques", control=False)
    folium.CircleMarker(ss.start or DEFAULT_CENTER, radius=0, opacity=0, fill_opacity=0,
                        class_name="mode-view" if locked() else
                        "mode-start" if click_mode == CLICK_MODES[0] else "mode-zone").add_to(fg)
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
        folium.Marker(ss.start, icon=folium.Icon(color="green", icon="play"),
                      interactive=False).add_to(fg)
        try:
            frame = LocalFrame(*ss.start)
            Lmax = v["distance"] * 1000 * ((1 + v["tol"] / 100) if v["mode"] == "max" else 1.3)
            for ring in _rings(build_region(make_params(v), frame, Lmax), frame):
                folium.Polygon([(la, lo) for lo, la in ring], color=ZONE_COLOR, weight=2,
                               dash_array="6", fill=True, fill_opacity=0.06).add_to(fg)
        except UserError as ex:
            zone_error = str(ex)
    res = ss.result
    profile = None
    if res is not None:
        loop, sel = shown_loop(res), selected(res)
        many = len(res.candidates) > 1
        for i, other in enumerate(res.candidates):      # les autres boucles, cliquables
            if i != sel:
                folium.PolyLine(list(zip(other.lat, other.lon)), color=loop_color(res, i),
                                weight=3.5, opacity=0.5, class_name="trail-cand",
                                bubbling_mouse_events=False).add_to(fg)
        folium.PolyLine(list(zip(loop.lat, loop.lon)), color=loop_color(res, sel),
                        weight=6 if many else 4).add_to(fg)
        if many:    # boucle affichée : pointillés blancs qui défilent le long du tracé
            folium.PolyLine(list(zip(loop.lat, loop.lon)), color="#fff", weight=2, dash_array="8 16",
                            class_name="trail-active", interactive=False).add_to(fg)
        if (res.debug.get("start_snap_m") or 0) > 30:
            folium.CircleMarker((loop.lat[0], loop.lon[0]), radius=7,
                                color=loop_color(res, sel), fill=True).add_to(fg)
        keep = np.unique(np.linspace(0, len(loop.dist) - 1, 800).astype(int))
        profile = {"d": np.round(loop.dist[keep] / 1000, 3).tolist(), "z": np.round(loop.ele[keep], 1).tolist(),
                   "lat": np.round(loop.lat[keep], 6).tolist(), "lon": np.round(loop.lon[keep], 6).tolist(),
                   "summary": (f"{loop.length / 1000:.2f} km · D+ {loop.dplus:.0f} m · "
                               f"{loop.dplus / loop.length * 1000:.0f} m/km")}
    ProfileLink(profile, touch).add_to(fg)  # toujours présent : sans résultat, il retire l'ancien profil
    out = st_folium(m, height=height, use_container_width=True, key="map", feature_group_to_add=fg,
                    center=ss.view[:2] if ss.view else None, zoom=ss.view[2] if ss.view else None,
                    returned_objects=["last_clicked", "last_object_clicked"])
    return out, zone_error


def handle_click(out, click_mode: str) -> None:
    """Un clic sur la carte place le départ ou ajoute un sommet de zone."""
    # Clic sur le tracé d'une autre boucle : elle passe au premier plan.
    obj = (out or {}).get("last_object_clicked")
    res = ss.result
    if obj and obj != ss.get("last_obj") and res is not None and len(res.candidates) > 1:
        ss.last_obj = obj
        sel = selected(res)
        best = (None, 3e-4)                             # ~30 m : au-delà, ce n'est pas un tracé
        for i, c in enumerate(res.candidates):
            if i != sel:
                d = float(np.hypot(c.lat - obj["lat"], (c.lon - obj["lng"]) * 0.66).min())
                if d < best[1]:
                    best = (i, d)
        if best[0] is not None:
            ss.cand = best[0]
            st.rerun()
    click = (out or {}).get("last_clicked")
    if click and click != ss.get("last_click"):
        ss.last_click = click
        if locked():        # boucles proposées : la carte ne pose plus de point
            return
        pt = (click["lat"], click["lng"])
        if click_mode == CLICK_MODES[0]:
            ss.start = pt
            ss.focus = "Calculer"
        else:
            ss.polygon = (ss.polygon or []) + [(pt[1], pt[0])]
        st.rerun()   # le tracé affiché reste en place jusqu'au prochain « Calculer »


# ---------------------------------------------------------------- scripts de page
SCROLL_JS = """
setTimeout(function () {
  // Après un calcul : la légende juste au-dessus du bas de l'écran si carte + résultats
  // tiennent dans la fenêtre ; sinon on cale la page sur la carte.
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
}, 400);"""


def inject_js(extra: list[str] | None = None) -> None:
    """Scripts de page, à appeler en fin de script : défilement après calcul, focus clavier
    (Entrée enchaîne recherche -> « Départ ici » -> « Calculer »), Échap = « Annuler »,
    géolocalisation, plus les scripts `extra` de la vue."""
    pending = list(extra or [])
    if ss.pop("scroll_to_result", False):
        pending.append(SCROLL_JS)
    focus = ss.pop("focus", None)
    if focus:
        pending.append("""
    setTimeout(function () {
      const b = Array.from(d.querySelectorAll('button')).find(x => x.innerText.trim() === %r);
      if (b && window.parent.matchMedia('(pointer: fine)').matches) b.focus();  // pas au doigt
    }, 350);""" % focus)
    if ss.pop("want_geo", False):
        pending.append(GEO_JS)
    page_js = """<script>
const d = window.parent.document;
if (!window.parent.__trailEscPage2) {
  // Installé dans la page elle-même (pas dans ce cadre, recréé à chaque rafraîchissement).
  window.parent.__trailEscPage2 = true;
  const sc = d.createElement('script');
  sc.textContent = `document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    const b = document.querySelector('.st-key-btn-cancel button');
    if (b) b.click();
  }, true);`;
  d.head.appendChild(sc);
}
%s
</script>""" % (("// %s" % time.time() + "".join(pending)) if pending else "")
    if hasattr(st, "iframe"):       # Streamlit récent ; components.html est obsolète
        st.iframe(page_js, height=1)
    else:
        components.html(page_js, height=0)
