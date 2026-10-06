// Types de l'API GET /api/plan (contracts/api.md v1), écrits à la main (I5 : pas d'OpenAPI).
export type Msg = { code: string; params?: Record<string, number | string | null>; suggest?: Record<string, number | string | boolean | null>; checked?: boolean };

export type Candidate = {
  length_m: number;
  dplus_m: number;
  feasible: boolean;
  alt_min_m: number;
  alt_max_m: number;
  max_grade_pct: number;
  /** api.md v1.7 : part (0–1) de la longueur sur chemin (le reste : route) ; absente des sorties plus anciennes */
  trail_frac?: number;
  target_gap: { distance_m: number; dplus_m: number } | null;
  climbs?: { count: number; longest_gain_m: number; longest_len_m: number; gbar_m: number; mean_grade_pct: number };
  lat: number[];
  lon: number[];
  ele: number[];
  dist: number[];
  /** HMAC du serveur (api.md v1.3, M3) : recopié tel quel dans POST /api/loops ; absent des boucles relues */
  sig?: string;
  /** api.md v1.5 : points de passage passés (ordre de passage), `dist_m` = abscisse sur la boucle */
  via?: { n: number; lat: number; lon: number; snap_m: number; dist_m: number }[];
  /** api.md v1.5 : cols et sommets traversés ; `ele_m` approchée (peut être null) ; `lat`/`lon` = position du repère */
  landmarks?: { kind: 'col' | 'summit'; name: string; ele_m: number | null; dist_m: number; lat: number; lon: number }[];
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
