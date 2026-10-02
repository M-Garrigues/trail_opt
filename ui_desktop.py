"""Vue bureau : réglages dans la barre latérale, grande carte, panneau de debug."""
from __future__ import annotations

import streamlit as st

import ui_common as ui
from ui_common import ss


def render() -> None:
    # Moins d'espace vide au-dessus du titre (6rem par défaut dans Streamlit).
    st.markdown("<style>.block-container { padding-top: 2.2rem; }</style>", unsafe_allow_html=True)

    with st.sidebar:
        st.header("Paramètres")
        v = ui.params_form()
        st.markdown('<a href="?vue=mobile" target="_self">Version mobile</a>', unsafe_allow_html=True)

    st.title("Boucle de trail à D+ optimal")
    st.write("Clique sur la carte pour placer le **départ**, puis, si tu veux, les sommets de la "
             "**zone** : elle se ferme toute seule dès 3 points. Sans zone : disque centré sur le départ.")

    ui.search_box()

    # Calcul : bouton sous la recherche dès qu'un départ est posé, barre de progression dessous.
    with st.container():
        ui.calc_block(v)

    # Barre d'outils de la carte : mode de clic à gauche, les trois actions d'effacement
    # regroupées à droite, alignées sur la même ligne.
    c1, c2, c3, c4 = st.columns([3.2, 1.5, 1.2, 1.2], vertical_alignment="bottom")
    click_mode = c1.radio("Un clic sur la carte place", ui.CLICK_MODES,
                          index=0 if ss.start is None else 1, horizontal=True)
    if c2.button("Annuler le dernier point", disabled=not ss.polygon, use_container_width=True):
        ss.polygon = ss.polygon[:-1] or None
        st.rerun()
    if c3.button("Effacer la zone", disabled=not ss.polygon, use_container_width=True):
        ss.polygon = None
        st.rerun()
    if c4.button("Tout effacer", disabled=not (ss.start or ss.polygon or ss.result),
                 use_container_width=True):
        ss.start = ss.polygon = ss.result = None
        st.rerun()

    out, zone_error = ui.render_map(v, click_mode, height=620)
    res = ss.result
    if res is not None:  # résultat juste sous la carte, GPX à droite
        n = 5 if "err_dplus" in res.debug else 3
        c = st.columns([1] * n + [1.3], vertical_alignment="center")
        c[0].metric("Distance", f"{res.length / 1000:.2f} km")
        c[1].metric("D+", f"{res.dplus:.0f} m")
        c[2].metric("D+/km", f"{res.dplus / res.length * 1000:.0f} m")
        if n == 5:
            c[3].metric("Écart distance", f"{res.debug['err_distance']:+.1%}")
            c[4].metric("Écart D+", f"{res.debug['err_dplus']:+.1%}")
        c[-1].download_button("Télécharger le GPX", res.gpx, file_name=f"boucle_{v['distance']:g}km.gpx",
                              mime="application/gpx+xml", type="primary", use_container_width=True)
        for w in res.warnings:
            st.warning(w)
    st.caption("Vert : départ. Pointillés bleus : zone utile (zone dessinée coupée au rayon atteignable). "
               "Gris : sommets dessinés. Rouge : boucle ; rond rouge : départ effectif s'il a été déplacé. "
               "Fonds et relief : bouton de couches en haut à droite de la carte. "
               "Survole le profil ou le tracé : le point correspondant s'affiche sur l'autre.")
    st.markdown('<div id="legend-end"></div>', unsafe_allow_html=True)
    ui.handle_click(out, click_mode)
    if zone_error:
        st.error(zone_error)

    if res is not None:
        debug_panel(res)
    ui.inject_js()


def debug_panel(res) -> None:
    d = res.debug
    with st.expander("Debug", expanded=True):
        t = d.get("timings_s", {})
        lines = [
            f"**Solveur** : {d.get('solver')} — {d.get('solver_reason')} — {res.method}",
            f"**Graphe** ({d.get('source')}) : {d.get('source_ways')} voies → {d.get('raw_edges')} tronçons → "
            f"{d.get('edges_in_zone')} dans la zone → {d.get('edges_simplified')} simplifiés → "
            f"{d.get('edges_pruned')} après ponts/portée → {d.get('edges_after_grade')} après pente ; "
            f"{d.get('nodes')} nœuds ; {d.get('edges_doubled_near_start')} arêtes doublées "
            f"à moins de 200 m du départ, {d.get('edges_used_twice_near_start')} parcourues deux fois ; "
            f"carrefours uniques : {'oui' if d.get('node_simple') else 'non'} ; "
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
