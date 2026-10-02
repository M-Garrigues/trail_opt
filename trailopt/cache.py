"""Cache disque éphémère (/tmp) partagé par OSM et altitude."""
from __future__ import annotations

import os
from pathlib import Path

CACHE_DIR = Path(os.environ.get("TRAILOPT_CACHE_DIR", "/tmp/trailopt_cache"))


def offline() -> bool:
    """TRAILOPT_OFFLINE=1 : tout appel réseau lève une erreur (tests)."""
    return os.environ.get("TRAILOPT_OFFLINE") == "1"


def path(*parts: str) -> Path:
    p = Path(os.environ.get("TRAILOPT_CACHE_DIR", str(CACHE_DIR))).joinpath(*parts)
    p.parent.mkdir(parents=True, exist_ok=True)
    return p


def write_atomic(p: Path, data: bytes) -> None:
    tmp = p.with_suffix(p.suffix + ".tmp")
    tmp.write_bytes(data)
    os.replace(tmp, p)


def new_stats() -> dict:
    return {"requests": 0, "cache_hits": 0}
