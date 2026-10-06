"""Fusion des shards : altitude de nœud harmonisée entre dalles ; seuils de `check` ; repli d'altitude."""
import hashlib
import json

import numpy as np

from pipeline.build import tile_columns
from pipeline.check import check, node_conflicts
from pipeline.load import read_tile
from pipeline.merge import harmonize
from trailopt import ign

IY = 342
Y0 = IY * 200_000
NODE = (33 * 200_000, Y0 + 1000)              # nœud sur la frontière des dalles 32 et 33 (dm)


def _tile(d, ix, troncons, sample):
    """troncons : (id, [(x_dm, y_dm), …], nature) ; écrit <ix>_342.npz, renvoie l'info du manifeste."""
    n = np.array([len(g) for _, g, _ in troncons], np.int32)
    xy = np.array([p for _, g, _ in troncons for p in g], np.int64)
    A = dict(n=n, nature=np.array([ign.NATURES.index(t) for *_, t in troncons], np.uint8),
             importance=np.zeros(len(n), np.uint8), flat=np.zeros(len(n), bool), ok=np.ones(len(n), bool),
             ident=np.array([f"TRONROUT{i:016d}" for i, _, _ in troncons], dtype="S24"))
    T, info = tile_columns(ix, IY, A, xy[:, 0], xy[:, 1], sample)
    np.savez_compressed(d / f"{ix}_{IY}.npz", **T)
    data = (d / f"{ix}_{IY}.npz").read_bytes()
    info.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data))
    return info


def _two_tiles(d, nature_b="Sentier", offset=3.0):
    """Sentier (dalle 32) et tronçon b (dalle 33) qui se touchent en NODE ; le MNT vu par la dalle 33 est
    décalé de `offset` m : le nœud partagé reçoit deux altitudes (cas des replis dépendant de la dalle)."""
    x, y = NODE
    tiles = {
        "32_342": _tile(d, 32, [(1, [(x - 3000, y), (x, y)], "Sentier")], lambda X, Y: 100.0 + X * 0 + Y * 0),
        "33_342": _tile(d, 33, [(2, [(x, y), (x + 2000, y + 500)], nature_b)],
                        lambda X, Y: 100.0 + offset + 0.01 * (X - x / 10)),
    }
    m = {"format": "tiles/1", "natures": ign.NATURES, "tiles": tiles}
    (d / "manifest.json").write_text(json.dumps(m))
    return m


def _end_z(d, k):
    t = read_tile(d / f"{k}.npz", *map(int, k.split("_")))
    return t["z"][0], t["z"][-1], t


def test_harmonize_unique_node_altitude_and_ramp(tmp_path):
    m = _two_tiles(tmp_path)
    assert node_conflicts(tmp_path, m["tiles"])                     # 100 m contre 103 m
    before = _end_z(tmp_path, "33_342")
    assert harmonize(tmp_path, m, backup=tmp_path / "avant", log=lambda s: None) == ["33_342"]
    (tmp_path / "manifest.json").write_text(json.dumps(m))
    assert (tmp_path / "avant" / "33_342.npz").exists()
    assert check(tmp_path) == []                                    # sha256 à jour, nœud unique
    z0, z1, t = _end_z(tmp_path, "33_342")
    assert z0 == 100.0 and z1 == before[1]                         # plus basse ; l'autre bout ne bouge pas
    q = np.round(t["z"] * 10).astype(int)
    assert t["dplus_dm"][0] - t["dminus_dm"][0] == q[-1] - q[0]     # D+/D− recalculés
    assert np.all(np.diff(q - np.round(before[2]["z"] * 10).astype(int)) >= 0)   # rampe monotone 0 -> 3 m
    assert harmonize(tmp_path, m, log=lambda s: None) == []         # idempotent


def test_harmonize_prefers_value_off_ferries(tmp_path):
    m = _two_tiles(tmp_path, "Bac ou liaison maritime", offset=-5.0)   # bac plus bas : ignoré
    harmonize(tmp_path, m, log=lambda s: None)
    assert _end_z(tmp_path, "33_342")[0] == 100.0


def _manifest(d, **info):
    m = _two_tiles(d, offset=0.0)
    m["tiles"]["32_342"].update(info)
    (d / "manifest.json").write_text(json.dumps(m))


def test_check_jumps_warn_then_fail(tmp_path, capsys):
    _manifest(tmp_path, n=1000, jump_gt10=11)          # > max(10, 0,5 %) : avertissement seul
    assert check(tmp_path) == [] and "jump_gt10" in capsys.readouterr().err
    _manifest(tmp_path, n=1000, jump_gt10=21)          # > max(10, 2 %) : erreur
    assert "jump_gt10" in check(tmp_path)[0]


def test_check_nodata_counts_off_ferries_and_size(tmp_path):
    _manifest(tmp_path, nodata_frac=0.9)               # 1 tronçon sans MNT : îlot, avertissement
    assert check(tmp_path) == []
    _manifest(tmp_path, nodata_frac=0.9, n=1000)       # n du manifeste ne compte pas : tronçons de la dalle
    assert check(tmp_path) == []


def test_check_nodata_fails_on_large_land(tmp_path, monkeypatch):
    import pipeline.check as c
    monkeypatch.setattr(c, "MIN_NODATA_N", 0.5)
    _manifest(tmp_path, nodata_frac=0.9)
    assert "nodata hors bacs" in check(tmp_path)[0]
    _manifest(tmp_path, nodata_frac=0.5)
    assert check(tmp_path) == []


def test_fallback_altitude_independent_of_tile():
    """Nœud sans MNT sur son tronçon : altitude de l'extrémité la plus proche (≤ 1 km) parmi TOUS les
    tronçons chargés, même hors de la dalle, donc la même vue des deux dalles."""
    x, y = NODE

    def sample(X, Y):                       # pas de MNT à moins de 300 m à l'ouest du nœud
        X = np.asarray(X, float)
        return np.where(X > x / 10 - 300, np.nan, 50.0 + 0 * X)

    tr = [(1, [(x - 2000, y), (x, y)], "Sentier"),          # dalle 32, nœud sans MNT en NODE
          (2, [(x - 4000, y + 50), (x - 2500, y + 50)], "Sentier"),   # dalle 32, MNT
          (3, [(x, y), (x + 900, y)], "Sentier")]            # dalle 33, sans MNT
    n = np.array([2, 2, 2], np.int32)
    xy = np.array([p for _, g, _ in tr for p in g], np.int64)
    A = dict(n=n, nature=np.full(3, ign.NATURES.index("Sentier"), np.uint8), importance=np.zeros(3, np.uint8),
             flat=np.zeros(3, bool), ok=np.ones(3, bool),
             ident=np.array([f"TRONROUT{i:016d}" for i, _, _ in tr], dtype="S24"))
    zs = []
    for ix in (32, 33):
        T, info = tile_columns(ix, IY, A, xy[:, 0], xy[:, 1], sample)
        assert info["node_fallback"] >= 1
        zs.append(T["prof_z0_dm"])
    assert zs[1][0] == 500 and zs[0].tolist().count(500) >= 1      # 50 m des deux côtés
