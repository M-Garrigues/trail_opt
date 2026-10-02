"""Vue mobile : recherche, distance et réglages en tête ; carte à la taille de l'écran ;
bouton Calculer fixé en bas ; profil et tracé utilisables au doigt."""
from __future__ import annotations

import streamlit as st

from . import common as ui
from .common import ss

CSS = """<style>
[data-testid="stSidebar"], [data-testid="stSidebarCollapsedControl"] { display: none; }
.block-container { padding: 3.1rem .7rem 7rem .7rem !important; }
.m-title { font-size: 1.15rem; font-weight: 700; margin: 0 0 .2rem 0; }
.m-title a { font-size: .8rem; font-weight: 400; float: right; margin-top: .25rem; }
.m-result { font-size: 1.15rem; font-weight: 700; margin: .1rem 0 .3rem 0; }
.stButton button, .stDownloadButton button { min-height: 46px; }
/* Barre d'action toujours visible en bas de l'écran. */
.st-key-calcbar { position: fixed; left: 0; right: 0; bottom: 0; z-index: 1000;
  background: var(--trail-bg, var(--st-background-color, #fff));   /* suit le thème clair/sombre */
  padding: .4rem .7rem calc(.4rem + env(safe-area-inset-bottom)) .7rem; gap: .3rem;
  box-shadow: 0 -1px 5px rgba(0,0,0,.12); }
.st-key-calcbar [data-testid="stCaptionContainer"] { text-align: center; margin: .35rem 0; }
/* Pendant le calcul : plus de bouton Calculer, juste la barre de progression et une croix. */
.st-key-calcbar:has([data-testid="stProgress"]) { flex-direction: row; align-items: center; gap: .5rem; }
.st-key-calcbar:has([data-testid="stProgress"]) .st-key-btn-calc { display: none; }
.st-key-calcbar [data-testid="stElementContainer"]:has([data-testid="stProgress"]) { flex: 1 1 auto; width: auto; }
.st-key-calcbar .st-key-btn-cancel { flex: 0 0 38px; width: 38px !important; }
.st-key-calcbar .st-key-btn-cancel button { min-height: 34px; height: 34px; width: 38px; padding: 0;
  border-radius: 50%; }
/* Rangées de boutons qui restent côte à côte (Streamlit empile les colonnes sous 640 px). */
[class*="st-key-row-cand-"] [data-testid="stHorizontalBlock"] { flex-wrap: nowrap; gap: .35rem; }
[class*="st-key-row-cand-"] [data-testid="stColumn"] { min-width: 0 !important; }
[class*="st-key-row-cand-"] [data-testid="stColumn"]:first-child { flex: 1 1 auto; }
[class*="st-key-row-cand-"] [data-testid="stColumn"]:first-child button p { font-size: .8rem; }
[class*="st-key-row-cand-"] [data-testid="stColumn"]:not(:first-child) { flex: 0 0 74px; }
[class*="st-key-row-cand-"] button { min-height: 40px; padding: 0 .3rem; }
.st-key-row-zone [data-testid="stHorizontalBlock"], .st-key-row-params [data-testid="stHorizontalBlock"],
.st-key-row-search [data-testid="stHorizontalBlock"] { flex-wrap: nowrap; gap: .4rem; }
.st-key-row-zone [data-testid="stColumn"], .st-key-row-params [data-testid="stColumn"],
.st-key-row-search [data-testid="stColumn"] { min-width: 0 !important; }
.st-key-row-params button { min-height: 40px; }
.st-key-row-search [data-testid="stColumn"]:last-child { flex: 0 0 54px; }
.stRadio { margin: -.35rem 0 -.5rem 0; }
[data-testid="stVerticalBlock"] { gap: .55rem; }
</style>"""

# La carte remplit l'écran entre le haut de page et la barre du bas : hauteur en CSS
# (le serveur ne connaît pas la taille de l'écran). TOP = ce qui précède la carte.
TOP_PX, BAR_PX = 262, 62
MAP_FRAME_CSS = f"""<style>
iframe[title="streamlit_folium.st_folium"] {{
  height: max(300px, calc(100dvh - {TOP_PX + BAR_PX}px)) !important; }}
</style>"""
# Dans le cadre de la carte : la carte occupe toute la hauteur du cadre.
MAP_INNER_CSS = """<style>
html, body, #parent, #map_div, #map_div2, .folium-map { height: 100vh !important; }
</style>"""


# Couleur de fond réelle de l'app (thème clair ou sombre) pour la barre fixée en bas.
THEME_JS = """
(function () {
  const app = d.querySelector('.stApp');
  if (app) d.documentElement.style.setProperty('--trail-bg', window.parent.getComputedStyle(app).backgroundColor);
})();"""


def render() -> None:
    st.markdown(CSS + MAP_FRAME_CSS, unsafe_allow_html=True)
    st.markdown('<div class="m-title">⛰️ Boucle trail D+'
                '<a href="?vue=bureau" target="_self">version bureau</a></div>',
                unsafe_allow_html=True)
    ui.search_box(compact=True)
    v = ui.params_form(compact=True)     # distance et réglages visibles d'entrée, au-dessus de la carte
    click_mode = st.radio("Toucher la carte place", ui.CLICK_MODES, label_visibility="collapsed", disabled=ui.locked(),
                          index=0 if ss.start is None else 1, horizontal=True,
                          format_func=lambda m: "Je place " + m)
    map_box, result_box = st.container(), st.container()
    with map_box:
        out, zone_error = ui.render_map(v, click_mode, height=420, touch=True, extra_css=MAP_INNER_CSS)
        if zone_error:
            st.error(zone_error)

    res = ss.result
    with result_box:
        if res is not None:
            if len(res.candidates) > 1:
                ui.candidate_list(res, v, compact=True)
            else:
                loop = res.candidates[0]
                line = (f"{loop.length / 1000:.2f} km · D+ {loop.dplus:.0f} m · "
                        f"{loop.dplus / loop.length * 1000:.0f} m/km")
                if "err_dplus" in res.debug:
                    line += f" · écart D+ {res.debug['err_dplus']:+.0%}"
                st.markdown(f'<div class="m-result">{line}</div>', unsafe_allow_html=True)
                st.download_button("Télécharger le GPX", loop.gpx, file_name=f"boucle_{v['distance']:g}km.gpx",
                                   mime="application/gpx+xml", use_container_width=True)
            for w in res.warnings:
                st.warning(w)
        if ss.start or ss.polygon:
            with st.container(key="row-zone"):
                c1, c2, c3 = st.columns(3)
                if c1.button("Annuler point", disabled=not ss.polygon, use_container_width=True):
                    ss.polygon = ss.polygon[:-1] or None
                    st.rerun()
                if c2.button("Effacer zone", disabled=not ss.polygon, use_container_width=True):
                    ss.polygon = None
                    st.rerun()
                if c3.button("Tout effacer", use_container_width=True):
                    ss.start = ss.polygon = ss.result = None
                    st.rerun()
        st.markdown('<div id="legend-end"></div>', unsafe_allow_html=True)

    st.caption("Vert : départ. Pointillés bleus : zone utile. Rouge : boucle. Glisse le doigt sur le "
               "profil, ou touche le tracé, pour situer un point. Fonds et relief : bouton en haut "
               "à droite de la carte.")

    with st.container(key="calcbar"):   # fixé en bas d'écran : ne décale rien dans la page
        ui.calc_block(v, fit={"width_px": 330, "height_px": 220}, full_width=True, cancel_label="✕")

    ui.handle_click(out, click_mode)
    ui.inject_js([THEME_JS])
