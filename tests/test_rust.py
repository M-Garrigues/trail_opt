"""Drapeau moteur Rust : boucle valide si le binaire existe, repli Python sinon."""
import pytest

from conftest import build, check_loop
from trailopt.solvers import optimize, rust


@pytest.mark.skipif(not rust.ENGINE.exists(), reason="moteur Rust non compilé (cargo build --release)")
@pytest.mark.parametrize("mode,node_simple", [("max", False), ("max", True), ("target", False)])
def test_rust_valid(mode, node_simple):
    P, _ = build(2400.0, mode=mode, D=120.0 if mode == "target" else None, n=9, node_simple=node_simple)
    res = optimize(P, budget=2.0, solver="rust", n_candidates=2, seed=1)
    assert res.method.startswith("moteur Rust") and "rust_repli" not in res.debug
    assert check_loop(P, res.circuit) == pytest.approx((res.length, res.dplus))
    for c, length, dplus in res.alternatives:
        assert check_loop(P, c) == pytest.approx((length, dplus))


def test_rust_fallback(monkeypatch, tmp_path):
    monkeypatch.setattr(rust, "ENGINE", tmp_path / "absent")
    monkeypatch.setenv("TRAILOPT_SOLVER", "rust")
    P, _ = build(2400.0)
    res = optimize(P, budget=1.0, seed=1)
    assert "FileNotFoundError" in res.debug["rust_repli"]
    check_loop(P, res.circuit)
