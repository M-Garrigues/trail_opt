// Repères (D40) : pictogrammes au trait (viewBox 20), couleur = currentColor.
export const LM_PATH = {
  summit: 'M3 16l5-9 3 5 2-3 4 7z', // pic
  col: 'M2 6l4 4 4 4 4-4 4-4M2 16h16', // selle entre deux sommets
} as const;

/** Sports (D70) : pictogrammes au trait (viewBox 24, trait 2, bouts ronds), couleur = currentColor. */
export const SPORT_PATH = {
  // coureur penché en avant, petit trait de pente sous les pieds (le distingue de la course sur route)
  trail: 'M15 2a2 2 0 1 0 0.01 0M13 7l-2 6M11 13l3 3-1 5M11 13l-4 2-3-1M13 7l-4 2-2 2M13 7l3 3 3-1M3 22l18-3',
  // vélo de profil neutre (ni VTT ni route) : deux roues, cadre droit, selle et guidon
  bike: 'M2 16a4 4 0 1 0 8 0a4 4 0 1 0-8 0M14 16a4 4 0 1 0 8 0a4 4 0 1 0-8 0M6 16l4-7 2 7zM10 9h6l-4 7M16 9l2 7M10 9l-.5-2M8 7h3M16 9l-.5-2.5H18',
} as const;
