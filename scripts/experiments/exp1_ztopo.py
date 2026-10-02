"""Exp. 1 : préparation rapide (Z BD TOPO, sans densification ni LiDAR) + qualité des Z BD TOPO.

    PYTHONPATH=. .venv/bin/python scripts/experiments/exp1_ztopo.py [massy|FILE LAT LON] [R_KM] [R_COMPARE_KM]
"""
import pickle
import sys

import numpy as np

from trailopt import graph
from trailopt.geo import LocalFrame

import common as C

args = sys.argv[1:]
if args and args[0] != "massy":
    path, lat, lon = args[0], float(args[1]), float(args[2])
    args = args[3:]
    tag = "alt"
else:
    path, (lat, lon) = C.MASSY_FILE, C.MASSY
    args = args[1:]
    tag = "massy"
R = float(args[0]) * 1000 if args else 25000.0
RC = float(args[1]) * 1000 if len(args) > 1 else 9900.0
Lmax = 2 * R * 1.05 if R < 25000 else 105000.0

T = C.Timer()
feats = C.load_feats(path)
T.lap("load_json")
frame = LocalFrame(lat, lon)
raw, nways = C.fast_raw(feats, frame, "minor", R)
del feats
T.lap("fast_raw")
raw = graph.contract_degree2(raw)
T.lap("contract")
m = len(raw)
ln = np.array([np.hypot(*np.diff(e[2][:, :2], axis=0).T).sum() for e in raw])
w = np.array([C.topo_w(e[2], e[3]) for e in raw])
zbad = sum(int(C.bad_z(e[2][:, 2]).any()) for e in raw)
T.lap("len_w")
# nœuds contigus + départ = nœud le plus proche de l'origine (prototype : pas d'insertion)
nodes, inv = np.unique(np.array([[e[0], e[1]] for e in raw]).ravel(), return_inverse=True)
u, v = inv[0::2], inv[1::2]
nxy = np.zeros((len(nodes), 2))
nxy[u] = np.array([e[2][0, :2] for e in raw])
nxy[v] = np.array([e[2][-1, :2] for e in raw])
keep = ln > 1e-6
s = int(np.argmin(np.hypot(nxy[:, 0], nxy[:, 1])))
g = C.G(u, v, ln, w, s)
g.nxy = nxy
alive = C.prune_mask(g, keep, Lmax)
T.lap("prune")
print(f"[{tag}] R={R/1000:g} km  tronçons retenus={nways}  après contraction={m}  après élagage={int(alive.sum())}"
      f"  sommets={sum(len(e[2]) for e in raw)}  arêtes avec Z invalide={zbad}")
print("temps (s):", T.T, " pic RSS (Mo):", round(C.rss_mb()))
pickle.dump(dict(u=u, v=v, len=ln, w=w, s=s, nxy=nxy, alive=alive, lat=lat, lon=lon,
                 geom=[e[2].astype(np.float32) for e in raw], flat=np.array([e[3] for e in raw])),
            open(C.OUT / f"{tag}_fast_{R/1000:g}.pkl", "wb"), protocol=4)

# --- comparaison Z BD TOPO / LiDAR sur les arêtes élaguées à moins de RC du départ
if RC <= 0:
    sys.exit()
idx = [i for i in np.flatnonzero(alive) if np.hypot(raw[i][2][:, 0], raw[i][2][:, 1]).max() <= RC]
sampler = C.cached_sampler(frame)
T = C.Timer()
edges = {}
for i in idx:
    r = graph.densify(raw[i][2][:, :2], 5.0)
    edges[i] = graph.Edge(int(u[i]), int(v[i]), r[0], r[1], raw[i][3])
gg = graph.Graph(edges)
T.lap("densify")
npts = graph.assign_elevation(gg, sampler)
T.lap("lidar")
wl = np.array([edges[i].w for i in idx])
wt, l = w[idx], ln[idx]
# LiDAR aux seuls sommets BD TOPO (pas de densification)
allv = np.vstack([raw[i][2][:, :2] for i in idx])
zv = sampler(allv)
T.lap("lidar_sommets")
wv, dz, off = [], [], 0
for i in idx:
    n = len(raw[i][2])
    z = zv[off:off + n]
    off += n
    zt = raw[i][2][:, 2]
    ok = np.isfinite(z) & ~C.bad_z(zt)
    dz.append((zt - z)[ok & (not raw[i][3])])
    z = np.where(np.isfinite(z), z, np.nanmean(z) if np.isfinite(z).any() else 0.0)
    wv.append(abs(z[-1] - z[0]) / 2 if raw[i][3] else np.abs(np.diff(z)).sum() / 2)
wv, dz = np.array(wv), np.concatenate(dz)
print(f"\ncomparaison sur {len(idx)} arêtes (rayon {RC/1000:g} km), {npts} points LiDAR 5 m, {len(allv)} sommets BD TOPO")
print("temps (s):", T.T)
print(f"Z BD TOPO - Z LiDAR aux sommets (hors ponts) : médiane {np.median(dz):+.2f} m, "
      f"|écart| médian {np.median(np.abs(dz)):.2f}, p90 {np.quantile(np.abs(dz), .9):.2f}, p99 {np.quantile(np.abs(dz), .99):.2f}")
for name, x in (("Z BD TOPO", wt), ("LiDAR aux sommets", wv)):
    r = np.corrcoef(x, wl)[0, 1]
    rr = np.corrcoef(x / np.maximum(l, 1), wl / np.maximum(l, 1))[0, 1]
    big = l > 50
    from scipy.stats import spearmanr
    rs = spearmanr((x / l)[big], (wl / l)[big])[0]
    print(f"  w {name:18s}: somme {x.sum():8.0f} m  (LiDAR 5 m : {wl.sum():.0f} m, rapport {x.sum()/wl.sum():.2f})"
          f"  corr w {r:.3f}  corr pente {rr:.3f}  Spearman pente (l>50 m) {rs:.3f}")
# recouvrement des arêtes « raides » : top q % de la longueur par pente
for q in (0.1, 0.2, 0.3):
    def top(x):
        o = np.argsort(-x / np.maximum(l, 1))
        return set(o[: np.searchsorted(np.cumsum(l[o]), q * l.sum())].tolist())
    a, b = top(wl), top(wt)
    print(f"  top {q:.0%} de la longueur par pente : recouvrement topo/LiDAR {len(a & b)/len(a):.2f}, "
          f"part du D+ LiDAR captée par la sélection topo {wl[list(b)].sum()/wl[list(a)].sum():.2f}")
pickle.dump(dict(idx=np.array(idx), w_lidar=wl, w_topo=wt, w_vert=wv, len=l), open(C.OUT / f"{tag}_zcmp.pkl", "wb"))
