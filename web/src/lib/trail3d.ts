// T36 (D34) : tracé de la vue 3D rendu AU-DESSUS du relief, plus drapé dessus.
// Une ligne MapLibre drapée est peinte dans la texture du terrain : en forte pente face à la caméra elle s'étire
// (« bave »). MapLibre 6 n'a pas de line-z-offset → couche custom WebGL : ruban de largeur constante à l'écran, testé en
// profondeur contre le relief ; les parties cachées par une crête restent visibles en transparence.
// Hauteur : le ruban doit coller à la surface AFFICHÉE, or celle-ci bouge pendant plusieurs secondes après le passage
// en 3D (plate, puis dalles grossières, puis fines). Donc :
//  1. tant que le relief de la vue n'est pas chargé, c'est la ligne drapée de MapLibre qui reste affichée : visible à
//     l'instant, toujours collée à la surface quel que soit son état (ni attente, ni lévitation) ;
//  2. relief chargé et caméra posée → UNE passe : altitudes du relief affiché (queryTerrainElevation, déjà ×exagération ;
//     à défaut les nôtres), le ruban remplace la ligne drapée au même endroit ;
//  3. ensuite une passe par arrêt de caméra seulement (le relief change de résolution avec le zoom) : rien sous 0,5 m,
//     transition de 200 ms au-delà de 3 px d'écran. Jamais de calcul par image ni par dalle reçue.
// Surélévation anti z-fighting : ~2,5 px d'écran convertis en mètres (pas de décalage fixe qui « flotte » en plaine).
import { MercatorCoordinate, type CustomLayerInterface, type CustomRenderMethodInput, type Map } from 'maplibre-gl';
import type { Candidate } from './types';

const VS = `#version 300 es
precision highp float;
uniform mat4 u_matrix; uniform vec2 u_half; uniform float u_width; uniform float u_lift; uniform float u_m;
in vec3 a_pos; in vec3 a_prev; in vec3 a_next; in float a_side;
vec4 P(vec3 p) { return u_matrix * vec4(p.xy, (p.z + u_lift) * u_m, 1.0); }
void main() {
  vec4 c = P(a_pos), pr = P(a_prev), nx = P(a_next);
  vec2 sc = c.xy / c.w * u_half, d1 = normalize(sc - pr.xy / pr.w * u_half + vec2(1e-6, 0.0)), d2 = normalize(nx.xy / nx.w * u_half - sc + vec2(1e-6, 0.0));
  vec2 s = d1 + d2, n = length(s) < 1e-3 ? vec2(-d1.y, d1.x) : normalize(vec2(-s.y, s.x));
  float miter = 1.0 / max(dot(n, vec2(-d1.y, d1.x)), 0.8);
  gl_Position = c + vec4(n * a_side * u_width * 0.5 * miter / u_half * c.w, 0.0, 0.0);
}`;
const FS = `#version 300 es
precision mediump float;
uniform vec4 u_color; out vec4 fragColor;
void main() { fragColor = u_color; }`;

const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);

export class Trail3D implements CustomLayerInterface {
  id = 'trail-3d';
  type = 'custom' as const;
  renderingMode = '3d' as const;
  private map!: Map;
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private buf: WebGLBuffer | null = null;
  private n = 0;
  private cand: Candidate | null = null;
  private origin = [0, 0];
  /** mètres → unités mercator (z de mainMatrix : conforme, comme x et y) */
  private mScale = 0;
  private loc: Record<string, WebGLUniformLocation | null> = {};
  /** altitudes (m, ×exagération) des sommets posés ; null = ruban pas encore posé (ligne drapée affichée) */
  private z: Float32Array | null = null;
  private xy: number[][] = [];
  private timer = 0;
  private raf = 0;
  /** couleur, exagération du relief, bascule ligne drapée ↔ ruban, largeur (px CSS), surélévation maximale (m) */
  constructor(public color: string, private exag: number, private onShown: (shown: boolean) => void = () => {}, private widthPx = 5, private lift = 6) {}

  /** erreur dans la 3D (pose du ruban, rendu) : l'appli repasse en 2D au lieu de casser la carte (2026-10-08) */
  onError: (e: unknown) => void = () => {};

  setColor(c: string) { if (c !== this.color) { this.color = c; this.map?.triggerRepaint(); } }
  /** au plus une passe par « arrêt » : les rafales d'événements (dalles, fin de mouvement) sont regroupées */
  private schedule = (e: object) => {
    const src = (e as { sourceId?: string }).sourceId;
    if (src && src !== 'dem') return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => { try { this.pass(); } catch (err) { this.onError(err); } }, 80);
  };
  onAdd(map: Map, gl: WebGL2RenderingContext) {
    this.map = map;
    map.on('idle', this.schedule); map.on('moveend', this.schedule); map.on('sourcedata', this.schedule);
    this.gl = gl;
    const sh = (type: number, src: string) => { const s = gl.createShader(type)!; gl.shaderSource(s, src); gl.compileShader(s); return s; };
    const p = gl.createProgram()!;
    gl.attachShader(p, sh(gl.VERTEX_SHADER, VS));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) { console.warn('trail3d', gl.getProgramInfoLog(p)); return; }
    this.prog = p;
    for (const u of ['u_matrix', 'u_half', 'u_width', 'u_lift', 'u_color', 'u_m']) this.loc[u] = gl.getUniformLocation(p, u);
    this.buf = gl.createBuffer();
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    const stride = 10 * 4;
    [['a_pos', 3, 0], ['a_prev', 3, 3], ['a_next', 3, 6], ['a_side', 1, 9]].forEach(([name, size, off]) => {
      const l = gl.getAttribLocation(p, name as string);
      gl.enableVertexAttribArray(l);
      gl.vertexAttribPointer(l, size as number, gl.FLOAT, false, stride, (off as number) * 4);
    });
    gl.bindVertexArray(null);
    this.setLine(this.cand, true);
  }

  /** Tracé à dessiner (null = rien). Coordonnées relatives au premier point : précision float32 au mètre près. */
  setLine(c: Candidate | null, force = false) {
    if (c === this.cand && !force) return;
    this.cand = c;
    cancelAnimationFrame(this.raf);
    this.z = null; this.n = 0; this.xy = [];
    if (!this.gl) return;
    this.onShown(false); // la ligne drapée du nouveau tracé, tout de suite
    if (c && c.lat.length > 1) {
      const m = c.lat.map((la, i) => MercatorCoordinate.fromLngLat([c.lon[i], la]));
      this.origin = [m[0].x, m[0].y];
      this.mScale = m[0].meterInMercatorCoordinateUnits();
      for (let i = 0; i < m.length; i++) {
        const x = m[i].x - this.origin[0], y = m[i].y - this.origin[1], last = this.xy[this.xy.length - 1];
        if (last && Math.hypot(x - last[0], y - last[1]) < this.mScale) continue; // points < 1 m : pas de joint dégénéré
        this.xy.push([x, y, i]);
      }
    }
    this.map.triggerRepaint();
    this.pass(); // relief déjà là (changement de boucle en 3D) : ruban immédiat
  }

  /** mètres par pixel d'écran au centre de la carte */
  private mpp() { return 78271.5 * Math.cos((this.map.getCenter().lat * Math.PI) / 180) / 2 ** this.map.getZoom(); }

  /** Pose (ou recale) le ruban sur le relief affiché, si celui-ci est chargé et la caméra posée. */
  private pass() {
    const c = this.cand, map = this.map;
    if (!this.gl || !c || this.xy.length < 2 || map.isMoving() || !map.getTerrain() || !map.isSourceLoaded('dem')) return;
    const to = Float32Array.from(this.xy, ([, , i]) => map.queryTerrainElevation([c.lon[i], c.lat[i]]) ?? c.ele[i] * this.exag);
    const from = this.z;
    if (!from) { this.upload(to); this.onShown(true); return; }
    let d = 0;
    for (let i = 0; i < to.length; i++) d = Math.max(d, Math.abs(to[i] - from[i]));
    if (d < 0.5) return;
    cancelAnimationFrame(this.raf);
    if (d < 3 * this.mpp() || matchMedia('(prefers-reduced-motion: reduce)').matches) { this.upload(to); return; }
    const t0 = performance.now();
    const tick = () => {
      const k = Math.min(1, (performance.now() - t0) / 200), e = k * (2 - k);
      this.upload(to.map((v, i) => from[i] + (v - from[i]) * e));
      if (k < 1) this.raf = requestAnimationFrame(tick);
    };
    tick();
  }

  private upload(z: Float32Array) {
    const gl = this.gl;
    if (!gl || !this.buf) return;
    this.z = z;
    const P = this.xy.map(([x, y], i) => [x, y, z[i]]);
    const N = P.length, d = new Float32Array(N * 2 * 10);
    for (let i = 0; i < N; i++) {
      const prev = i ? P[i - 1] : P[0].map((v, k) => 2 * v - P[1][k]);
      const next = i < N - 1 ? P[i + 1] : P[N - 1].map((v, k) => 2 * v - P[N - 2][k]);
      for (const [j, side] of [[0, -1], [1, 1]]) d.set([...P[i], ...prev, ...next, side], (2 * i + j) * 10);
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    gl.bufferData(gl.ARRAY_BUFFER, d, gl.DYNAMIC_DRAW);
    this.n = N * 2;
    this.map.triggerRepaint();
  }

  render(gl: WebGL2RenderingContext, o: CustomRenderMethodInput) {
    try { this.draw(gl, o); } catch (err) { this.n = 0; this.onError(err); }
  }

  private draw(gl: WebGL2RenderingContext, o: CustomRenderMethodInput) {
    if (!this.prog || !this.n) return;
    // mainMatrix projette (x, y, z) mercator 0..1 (z conforme : mètres × u_m) ; on y ajoute la translation de l'origine (en float64)
    const M = Array.from(o.defaultProjectionData.mainMatrix as ArrayLike<number>);
    for (let r = 0; r < 4; r++) M[12 + r] += M[r] * this.origin[0] + M[4 + r] * this.origin[1];
    gl.useProgram(this.prog);
    gl.bindVertexArray(this.vao);
    gl.uniformMatrix4fv(this.loc.u_matrix, false, new Float32Array(M));
    gl.uniform2f(this.loc.u_half, gl.drawingBufferWidth / 2, gl.drawingBufferHeight / 2);
    gl.uniform1f(this.loc.u_m, this.mScale);
    // surélévation = ~2,5 px d'écran en mètres (au centre de la carte), bornée : ≥ 0,5 m, ≤ lift × exag
    gl.uniform1f(this.loc.u_lift, Math.min(this.lift * this.exag, Math.max(0.5, 2.5 * this.mpp())));
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA); // alpha prémultiplié (MapLibre)
    const dpr = devicePixelRatio || 1, [r, g, b] = rgb(this.color);
    const pass = (w: number, c: number[], depth: boolean) => {
      if (depth) { gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL); } else gl.disable(gl.DEPTH_TEST);
      gl.depthMask(false);
      gl.uniform1f(this.loc.u_width, w * dpr);
      gl.uniform4f(this.loc.u_color, c[0], c[1], c[2], c[3]);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, this.n);
    };
    const a = 0.35; // parties cachées par le relief : en transparence
    pass(this.widthPx, [r * a, g * a, b * a, a], false);
    pass(this.widthPx + 4, [1, 1, 1, 1], true);
    pass(this.widthPx, [r, g, b, 1], true);
    gl.bindVertexArray(null);
  }

  onRemove() {
    const m = this.map;
    m.off('idle', this.schedule); m.off('moveend', this.schedule); m.off('sourcedata', this.schedule);
    clearTimeout(this.timer); cancelAnimationFrame(this.raf);
    this.prog = null; this.gl = null; this.n = 0; this.z = null;
  }
}
