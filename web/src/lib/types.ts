// Types de l'API GET /api/plan (contracts/api.md v1), écrits à la main (I5 : pas d'OpenAPI).
export type Msg = { code: string; params?: Record<string, number | string | null> };

export type Candidate = {
  length_m: number;
  dplus_m: number;
  feasible: boolean;
  alt_min_m: number;
  alt_max_m: number;
  max_grade_pct: number;
  target_gap: { distance_m: number; dplus_m: number } | null;
  climbs?: { count: number; longest_gain_m: number; longest_len_m: number; gbar_m: number; mean_grade_pct: number };
  lat: number[];
  lon: number[];
  ele: number[];
  dist: number[];
  /** HMAC du serveur (api.md v1.3, M3) : recopié tel quel dans POST /api/loops ; absent des boucles relues */
  sig?: string;
};

export type PlanResponse = {
  solver_version: string;
  data_version: string;
  compute_s: number;
  lower_bound_m: number | null;
  effective_start: { lat: number; lon: number; kind: 'clicked' | 'moved' | 'access'; moved_m?: number; access_m?: number };
  zone?: { geometry: GeoJSON.Geometry; area_km2: number; reduced_radius_km: number | null } | null;
  candidates: Candidate[];
  warnings: Msg[];
};
