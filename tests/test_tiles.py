"""Dalles tiles/1 : écriture puis lecture d'une petite dalle synthétique, hors ligne."""
import json

import numpy as np
import pytest
from pyproj import Transformer

from pipeline.build import tile_columns
from pipeline.load import TileStore, profile_counts, read_tile
from trailopt import ign
from trailopt.geo import LocalFrame

IX, IY = 32, 342
X0, Y0 = IX * 200_000, IY * 200_000           # origine de dalle (dm)
# (id, sommets en dm relatifs à l'origine, pont/tunnel) ; ids volontairement non triés
TRONCONS = [
    (30, [(1000, 1000), (3000, 1000)], False),
    (10, [(1000, 1050), (3000, 1050)], False),              # parallèle à 30, 5 m au nord
    (20, [(3000, 1000), (3500, 2500), (3000, 4000)], True),  # part de la fin de 30, pont
    (40, [(-500, 1000), (1000, 1000)], False),              # 1er sommet hors dalle : dalle voisine
]


def relief(X, Y):
    return 100.0 + 0.01 * (X - X0 / 10) + 5.0 * np.sin(Y / 50.0)


def _arrays():
    n = np.array([len(g) for _, g, _ in TRONCONS], np.int32)
    xy = np.array([p for _, g, _ in TRONCONS for p in g], np.int64)
    A = dict(n=n, nature=np.full(len(n), ign.NATURES.index("Sentier"), np.uint8),
             importance=np.zeros(len(n), np.uint8), flat=np.array([f for *_, f in TRONCONS]),
             ok=np.ones(len(n), bool),
             ident=np.array([f"TRONROUT{i:016d}" for i, _, _ in TRONCONS], dtype="S24"))
    return A, xy[:, 0] + X0, xy[:, 1] + Y0


@pytest.fixture
def tile_dir(tmp_path):
    A, Xd, Yd = _arrays()
    T, info = tile_columns(IX, IY, A, Xd, Yd, relief)
    np.savez_compressed(tmp_path / f"{IX}_{IY}.npz", **T)
    (tmp_path / "manifest.json").write_text(json.dumps(
        {"format": "tiles/1", "tiles": {f"{IX}_{IY}": info}}))
    return tmp_path, info


def test_roundtrip(tile_dir):
    d, info = tile_dir
    assert info["n"] == 3 and info["nodata_frac"] == 0
    T = read_tile(d / f"{IX}_{IY}.npz", IX, IY)
    assert T["id"].tolist() == [10, 20, 30]                 # rangés par id, 40 exclu
    src = {i: g for i, g, _ in TRONCONS}
    off = np.concatenate([[0], np.cumsum(T["n"])])
    for k, i in enumerate(T["id"].tolist()):
        got = np.column_stack([T["x_dm"], T["y_dm"]])[off[k]:off[k + 1]] - (X0, Y0)
        assert got.tolist() == [list(p) for p in src[i]]
    assert T["pn"].tolist() == profile_counts(T["len_dm"]).tolist()
    assert T["len_dm"].tolist() == [2000, round(2 * np.hypot(500, 1500)), 2000]
    # D+ - D- = z(v) - z(u), et altitude unique au nœud partagé (fin de 30 = début de 20)
    poff = np.concatenate([[0], np.cumsum(T["pn"])])
    zdm = np.round(T["z"] * 10).astype(int)
    for k in range(3):
        a, b = poff[k], poff[k + 1] - 1
        assert T["dplus_dm"][k] - T["dminus_dm"][k] == zdm[b] - zdm[a]
    assert zdm[poff[3] - 1] == zdm[poff[1]]
    # pont : profil linéaire, donc monotone
    assert T["dplus_dm"][1] + T["dminus_dm"][1] == abs(zdm[poff[2] - 1] - zdm[poff[1]])
    # parallèles 10 <-> 30, par id global
    assert T["par_n"].tolist() == [1, 0, 1] and T["par_id"].tolist() == [30, 10]


def test_store_feeds_trailopt(tile_dir):
    d, _ = tile_dir
    store = TileStore(d)
    to_wgs = Transformer.from_crs("EPSG:2154", "EPSG:4326", always_xy=True)
    lon, lat = to_wgs.transform([X0 / 10 + 50, X0 / 10 + 450], [Y0 / 10 + 50, Y0 / 10 + 450])
    bbox = (lat[0], lon[0], lat[1], lon[1])
    assert store.count(bbox) == 3
    A = store.fetch(bbox)
    frame = LocalFrame(lat[0], lon[0])
    raw, n = ign.to_edges(A, frame, "unpaved")
    assert n == 3 and len({u for u, *_ in raw} | {v for _, v, *_ in raw}) == 5
    # altitude : exacte aux points du profil, interpolée entre deux
    T = store.tile(f"{IX}_{IY}")
    pts = np.array([[X0 / 10 + 100, Y0 / 10 + 105], [X0 / 10 + 102.5, Y0 / 10 + 105]])
    z = store.sample_l93(pts[:, 0], pts[:, 1])
    assert z[0] == pytest.approx(T["z"][0], abs=1e-9)
    assert z[1] == pytest.approx((T["z"][0] + T["z"][1]) / 2, abs=1e-9)
    sample = store.sampler_for(frame)
    assert sample.smooth == 1
    assert sample(frame.to_local(*to_wgs.transform(pts[:, 0], pts[:, 1])))[0] == pytest.approx(z[0], abs=1e-3)


def test_unknown_format_rejected(tmp_path):
    (tmp_path / "manifest.json").write_text('{"format": "tiles/0", "tiles": {}}')
    with pytest.raises(RuntimeError, match="format"):
        TileStore(tmp_path)
