"""Boucle de trail à D+ optimal : point d'entrée Streamlit.

Deux mises en page pour une seule app (même cache, même verrou de calcul) :
bureau (ui/desktop.py) et mobile (ui/mobile.py), choisies selon l'appareil.
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

from ui import common  # noqa: E402  (après set_page_config)

common.init_state()
if VIEW == "mobile":
    from ui import mobile
    mobile.render()
else:
    from ui import desktop
    desktop.render()
