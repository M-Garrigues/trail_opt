"""Boucle de trail à D+ optimal : point d'entrée Streamlit.

Deux mises en page pour une seule app (même cache, même verrou de calcul) :
bureau (ui_desktop) et mobile (ui_mobile), choisies selon l'appareil.
`?vue=mobile` ou `?vue=bureau` force l'une ou l'autre.
"""
from __future__ import annotations

import re

import streamlit as st


def pick_view() -> str:
    forced = st.query_params.get("vue")
    if forced in ("mobile", "bureau"):
        return forced
    try:
        agent = st.context.headers.get("User-Agent", "")
    except Exception:
        agent = ""
    return "mobile" if re.search(r"Mobi|Android|iPhone|iPod", agent) else "bureau"


VIEW = pick_view()
st.set_page_config(page_title="Boucle trail D+", page_icon="⛰️",
                   layout="wide" if VIEW == "bureau" else "centered",
                   initial_sidebar_state="auto" if VIEW == "bureau" else "collapsed")

import ui_common  # noqa: E402  (après set_page_config)

ui_common.init_state()
if VIEW == "mobile":
    import ui_mobile
    ui_mobile.render()
else:
    import ui_desktop
    ui_desktop.render()
