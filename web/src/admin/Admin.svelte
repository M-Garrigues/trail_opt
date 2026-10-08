<script lang="ts">
  // Page /admin (contracts/admin.md § 5) : français seulement, pensée pour ordinateur (empilée en mobile),
  // clé saisie gardée en sessionStorage, données de GET /api/admin/stats (Logs Insights).
  import { onMount } from 'svelte';
  import Chart from './Chart.svelte';
  import Heat from './Heat.svelte';
  import { DASH, DIMS, EVENTS, days, eventTotal, fmt, histBins, keyLabel, label, pct, perDay, preset, qty, shares, tiles, usd, type Stats, type Usage } from './stats';
  import { errorText } from '../i18n/format';

  const KEY = 'optrail.admin';
  const EXPIRED = 'aws credentials expired';
  const PERIODS = [[7, '7 j'], [30, '30 j'], [90, '90 j'], [400, '13 mois']] as const;
  const C = { a: '#a8441c', b: '#1565C0', ok: '#2e7d32', fail: '#c62828', rej: '#9e9e9e' };
  const EV_COLORS = ['#f0a581', '#a8441c', '#1565C0', '#6A1B9A'];

  let key = $state(sessionStorage.getItem(KEY) ?? '');
  let input = $state('');
  let range = $state(preset(30));
  let stats = $state<Stats | null>(null);
  let error = $state('');
  let loading = $state(false);
  let usage = $state<Usage | null>(null);
  let usageError = $state('');

  // consommation AWS : indépendante de la période (mois en cours), cache serveur de 3 h
  async function loadUsage() {
    usageError = '';
    try {
      const res = await fetch('/api/admin/usage', { headers: { 'x-admin-key': key } });
      const body = await res.json().catch(() => null);
      if (res.ok) usage = body as Usage;
      else usageError = body?.error?.detail === EXPIRED ? 'Identifiants AWS expirés : aws login --profile optrail' : `Erreur ${res.status}${body?.error?.detail ? ` : ${body.error.detail}` : ''}`;
    } catch {
      usageError = 'Serveur injoignable.';
    }
  }

  async function load() {
    if (!key) return;
    loading = true;
    error = '';
    if (!usage) void loadUsage();
    try {
      const res = await fetch(`/api/admin/stats?from=${range.from}&to=${range.to}`, { headers: { 'x-admin-key': key } });
      const body = await res.json().catch(() => null);
      if (res.status === 401) {
        sessionStorage.removeItem(KEY);
        key = '';
        error = 'Clé refusée.';
      } else if (res.status === 429) error = 'Trop d’essais, réessayer dans 15 min.';
      else if (body?.error?.detail === EXPIRED) error = 'Identifiants AWS expirés : aws login --profile optrail';
      else if (!res.ok) error = `Erreur ${res.status}${body?.error?.detail ? ` : ${body.error.detail}` : ''}`;
      else stats = body as Stats;
    } catch {
      error = 'Serveur injoignable.';
    }
    loading = false;
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    key = input.trim();
    input = '';
    sessionStorage.setItem(KEY, key);
    void load();
  }

  onMount(() => {
    document.title = 'optrail — admin';
    document.documentElement.lang = 'fr';
    const m = document.createElement('meta');
    m.name = 'robots';
    m.content = 'noindex';
    document.head.append(m);
    void load();
  });

  const ds = $derived(stats ? days(stats.from, stats.to) : []);
  const short = (d: string) => `${d.slice(8, 10)}/${d.slice(5, 7)}`;
  const tl = $derived(stats ? tiles(stats) : null);
  const visitors = $derived(stats && perDay(ds, stats.visitors_by_day, (r) => r.visitors));
  const visits = $derived(stats && perDay(ds, stats.visitors_by_day, (r) => r.hits));
  const calcs = $derived(stats?.calcs_by_day ? {
    ok: perDay(ds, stats.calcs_by_day, (r) => r.ok)!,
    fail: perDay(ds, stats.calcs_by_day, (r) => r.n - r.ok)!,
    rej: perDay(ds, stats.calcs_by_day, (r) => r.rejected)!,
  } : null);
  const perf = $derived(stats?.compute_s?.by_day ? {
    p50: perDay(ds, stats.compute_s.by_day, (r) => r.p50, NaN)!,
    p95: perDay(ds, stats.compute_s.by_day, (r) => r.p95, NaN)!,
  } : null);
  const events = $derived(stats?.events_by_day ? EVENTS.map(([e, name], i) => ({ name, color: EV_COLORS[i], values: perDay(ds, stats!.events_by_day, (r) => r[e] ?? 0)! })) : null);
  const hk = $derived(histBins(stats?.hist_km ?? null));
  const hd = $derived(histBins(stats?.hist_dplus_m ?? null));
  const mixes = $derived(stats ? [
    ['Type de sortie', shares(stats.goals, (r) => r.goal, (r) => r.n)],
    ['Type de voie', shares(stats.surfaces, (r) => r.surface, (r) => r.n)],
    ['Montées', shares(stats.climbs, (r) => r.climbs, (r) => r.n)],
    ['Appareils (visites)', shares(stats.devices, (r) => r.dev, (r) => r.visitors)],
  ] as const : []);
</script>

<main>
  <header>
    <h1>optrail — admin</h1>
    {#if key}
      <nav aria-label="Période">
        {#each PERIODS as [n, l]}
          <button class="btn small secondary" aria-pressed={range.to === preset(n).to && range.from === preset(n).from} onclick={() => { range = preset(n); void load(); }}>{l}</button>
        {/each}
        <label>du <input type="date" bind:value={range.from} max={range.to} /></label>
        <label>au <input type="date" bind:value={range.to} min={range.from} /></label>
        <button class="btn small primary" onclick={load} disabled={loading}>{loading ? 'Chargement…' : 'Actualiser'}</button>
        <button class="btn small link" onclick={() => { sessionStorage.removeItem(KEY); key = ''; stats = null; usage = null; }}>Oublier la clé</button>
      </nav>
    {/if}
  </header>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if !key}
    <form onsubmit={submit}>
      <label for="k">Clé d’administration</label>
      <input id="k" type="password" autocomplete="current-password" bind:value={input} required />
      <button class="btn primary" type="submit">Entrer</button>
    </form>
  {:else if stats && tl}
    <p class="meta">
      Du {stats.from} au {stats.to} · {fmt(stats.scanned_mb, 1)} Mo analysés
      {#if stats.incomplete} · <strong>résultats incomplets</strong> (requête trop longue : « — »){/if}
    </p>
    <section class="tiles" aria-label="Chiffres clés">
      <div><span>Visiteurs</span><b>{fmt(tl.visitors)}</b><small>visiteurs uniques par jour, cumulés</small></div>
      <div><span>Visites</span><b>{fmt(tl.visits)}</b><small>pages ouvertes</small></div>
      <div><span>Calculs</span><b>{fmt(tl.calcs)}</b><small>{fmt(tl.rejected)} rejetés avant calcul</small></div>
      <div><span>Taux d’échec</span><b>{pct(tl.failRate)}</b><small>calculs sans sortie</small></div>
      <div><span>Temps p50 / p95</span><b>{fmt(tl.p50, 1)} / {fmt(tl.p95, 1)} s</b><small>calculs réussis</small></div>
      <div><span>Partages</span><b>{fmt(eventTotal(stats, 'share_created'))}</b><small>liens créés · {fmt(eventTotal(stats, 'shared_open'))} ouverts</small></div>
      <div><span>GPX</span><b>{fmt(eventTotal(stats, 'gpx'))}</b><small>exports</small></div>
    </section>

    <section class="grid">
      <div class="card">
        {#if visitors && visits}
          <Chart title="Visiteurs et visites par jour" labels={ds.map(short)} series={[{ name: 'Visiteurs', color: C.a, values: visitors }, { name: 'Visites', color: C.b, values: visits }]} />
        {:else}<p>Visiteurs par jour : {DASH}</p>{/if}
      </div>
      <div class="card">
        {#if calcs}
          <Chart kind="bar" title="Calculs par jour" labels={ds.map(short)} series={[{ name: 'Réussis', color: C.ok, values: calcs.ok }, { name: 'Échecs', color: C.fail, values: calcs.fail }, { name: 'Rejetés', color: C.rej, values: calcs.rej }]} />
        {:else}<p>Calculs par jour : {DASH}</p>{/if}
      </div>
      <div class="card">
        {#if perf}
          <Chart title="Temps de calcul par jour (s)" digits={1} labels={ds.map(short)} series={[{ name: 'p50', color: C.a, values: perf.p50 }, { name: 'p95', color: C.b, values: perf.p95 }]} />
        {:else}<p>Temps de calcul par jour : {DASH}</p>{/if}
      </div>
      <div class="card">
        <h2>Échecs par code</h2>
        {#if stats.codes?.length}
          <table>
            <thead><tr><th>Code</th><th>Message</th><th class="n">Calculs</th><th class="n">Taux</th></tr></thead>
            <tbody>{#each stats.codes as c}<tr><td><code>{c.code}</code></td><td>{errorText('fr', c.code)}</td><td class="n">{fmt(c.n)}</td><td class="n">{pct(c.rate)}</td></tr>{/each}</tbody>
          </table>
        {:else}<p class="none">{stats.codes ? 'Aucun échec.' : DASH}</p>{/if}
        <h3>Rejetés avant calcul</h3>
        {#if stats.rejected_codes?.length}
          <table><tbody>{#each stats.rejected_codes as c}<tr><td><code>{c.code}</code></td><td class="n">{fmt(c.n)}</td></tr>{/each}</tbody></table>
        {:else}<p class="none">{stats.rejected_codes ? 'Aucun.' : DASH}</p>{/if}
      </div>
      <div class="card">
        {#if hk}<Chart kind="bar" title="Distance obtenue (km)" labels={hk.labels} series={[{ name: 'Calculs', color: C.a, values: hk.values }]} />{:else}<p>Distance : {DASH}</p>{/if}
      </div>
      <div class="card">
        {#if hd}<Chart kind="bar" title="D+ obtenu (m)" labels={hd.labels} series={[{ name: 'Calculs', color: C.b, values: hd.values }]} />{:else}<p>D+ : {DASH}</p>{/if}
      </div>
      {#each mixes as [title, rows]}
        <div class="card">
          <h2>{title}</h2>
          {#if rows?.length}
            <table class="bars"><tbody>{#each rows as r}<tr><td>{r.label}</td><td class="bar"><span style="width: {r.share * 100}%"></span></td><td class="n">{fmt(r.n)}</td><td class="n">{pct(r.share)}</td></tr>{/each}</tbody></table>
          {:else}<p class="none">{rows ? 'Aucune donnée.' : DASH}</p>{/if}
        </div>
      {/each}
      <div class="card">
        {#if events}
          <Chart kind="bar" title="Actions par jour" labels={ds.map(short)} series={events} />
        {:else}<p>Actions par jour : {DASH}</p>{/if}
      </div>
      <div class="card wide">
        <h2>Taux d’action <small>(actions rapportées aux calculs réussis ; distance et D+ : sortie 1 du calcul, sortie choisie pour l’action)</small></h2>
        {#if stats.action_rates}
          <div class="rates">
            {#each DIMS as [dim, title]}
              <table>
                <caption>{title}</caption>
                <thead><tr><th></th><th class="n">Calculs</th><th class="n">Partages</th><th class="n">%</th><th class="n">GPX</th><th class="n">%</th></tr></thead>
                <tbody>
                  {#each stats.action_rates[dim] as r}
                    <tr><td>{keyLabel(dim, r.key)}</td><td class="n">{fmt(r.calcs)}</td><td class="n">{fmt(r.share)}</td><td class="n">{pct(r.share_rate)}</td><td class="n">{fmt(r.gpx)}</td><td class="n">{pct(r.gpx_rate)}</td></tr>
                  {:else}<tr><td colspan="6" class="none">Aucune donnée.</td></tr>{/each}
                </tbody>
              </table>
            {/each}
          </div>
        {:else}<p class="none">{DASH}</p>{/if}
      </div>
      <div class="card wide">
        <h2>Combinaisons qui amènent le plus d’actions</h2>
        {#if stats.top_combos?.length}
          <table>
            <thead><tr><th>Type</th><th>Voie</th><th>Distance</th><th>D+</th><th>Rang</th><th class="n">Partages</th><th class="n">GPX</th></tr></thead>
            <tbody>{#each stats.top_combos as c}<tr><td>{label(c.goal)}</td><td>{label(c.surface)}</td><td>{keyLabel('km_bin', c.km_bin)}</td><td>{keyLabel('dplus_bin', c.dplus_bin)}</td><td>{keyLabel('rank', c.rank)}</td><td class="n">{fmt(c.share)}</td><td class="n">{fmt(c.gpx)}</td></tr>{/each}</tbody>
          </table>
        {:else}<p class="none">{stats.top_combos ? 'Aucune action enregistrée sur la période.' : DASH}</p>{/if}
      </div>
      <div class="card wide">
        <h2>Départs <small>(densité, cellules de ~500 m ; rouge : hors couverture)</small></h2>
        <Heat starts={stats.starts ?? []} outside={stats.starts_outside ?? []} />
        {#if !stats.starts?.length && !stats.starts_outside?.length}<p class="none">Aucun départ enregistré sur la période (champ absent des anciennes lignes).</p>{/if}
      </div>
      <div class="card wide" role="region" aria-label="Consommation AWS">
        <h2>Consommation AWS {#if usage}<small>({usage.month}, renouvellement le {usage.renewal} ; {fmt(usage.elapsed * 100)} % du mois écoulé ; lu à {new Date(usage.fetched_at * 1000).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })}, gardé 3 h)</small>{/if}</h2>
        {#if usageError}<p class="error">{usageError}</p>{/if}
        {#if usage}
          <div class="tiles usage">
            <div><span>Dépense du mois</span><b>{usd(usage.spend?.actual)}</b><small>coût brut hors crédits (AWS Budgets)</small></div>
            <div><span>Prévision fin de mois</span><b>{usd(usage.spend?.forecast)}</b><small>{usage.spend?.forecast_source === 'budgets' ? 'prévision AWS Budgets' : 'au rythme actuel'}</small></div>
            <div><span>Prochain palier</span><b>{usd(usage.spend?.next_step)}</b><small>paliers {usage.spend?.steps.map((x) => `${x} $`).join(' · ') ?? DASH} : chacun met l’API de calcul en pause (reprise manuelle)</small></div>
            <div><span>Crédits restants</span><b>{usd(usage.plan?.credits_usd)}</b><small>plan {usage.plan?.type === 'PAID' ? 'payant' : (usage.plan?.type ?? DASH)} : les crédits paient la facture</small></div>
          </div>
          <div class="scroll"><table>
            <thead><tr><th>Service</th><th>Poste</th><th class="n">Consommé ce mois</th><th class="n">Seuil gratuit / mois</th><th class="n">Part du seuil</th><th class="n">Projection fin de mois</th><th class="n">Coût du mois</th><th class="n">Coût prévu</th></tr></thead>
            <tbody>
              {#each usage.lines as l}
                <tr>
                  <td>{l.service}</td><td>{l.item}{#if l.now != null} <small>({qty(l.now, l.unit === 'Go-mois' ? 'Go' : l.unit)} stockés)</small>{/if}</td>
                  <td class="n">{qty(l.used, l.unit)}</td>
                  <td class="n">{l.free == null ? '' : l.free ? qty(l.free, l.unit) : 'aucun'}</td>
                  <td class="n">{pct(l.share)}</td>
                  <td class="n">{qty(l.projected, l.unit)}{#if l.projected_share != null} ({pct(l.projected_share)}){/if}</td>
                  <td class="n">{usd(l.cost)}</td><td class="n">{usd(l.cost_forecast)}</td>
                </tr>
              {/each}
            </tbody>
          </table></div>
          {#if usage.errors.length}<p class="error">Sources en échec : {usage.errors.join(' ; ')}</p>{/if}
          <p class="none">
            Sources, toutes gratuites et en lecture : métriques CloudWatch (requêtes et données CloudFront, journaux reçus, taille des buckets S3),
            lignes REPORT de la Lambda via Logs Insights (requêtes et durée facturée, démarrages compris), taille des journaux, AWS Budgets
            (dépense réelle, coût brut hors crédits, et sa prévision quand AWS en donne une) et le plan du compte (crédits restants).
            Le compte est au plan payant ouvert après juillet 2025 : pas d’offre gratuite de 12 mois (l’API Free Tier ne renvoie aucun poste),
            seulement les offres « toujours gratuites » (Lambda, CloudFront, CloudWatch Logs) dont les seuils sont rappelés ici ; S3 est payant
            dès le premier octet. Coûts des postes = consommation au-delà du seuil × prix public eu-north-1 (relevé le 8 octobre 2026) ;
            « Non ventilé » = dépense réelle − postes estimés (surtout les requêtes S3, non mesurables gratuitement : envoi des dalles, partages).
            Les 5 Go gratuits de CloudWatch Logs sont communs à l’ingestion, au stockage et à l’analyse. Projection = consommé ÷ part du mois écoulée
            (stockage : taille actuelle), peu fiable en début de mois. Cost Explorer (détail exact par poste) n’est pas utilisé : 0,01 $ par appel,
            plus que la facture actuelle.
          </p>
        {:else if !usageError}<p class="none">Chargement de la consommation…</p>{/if}
      </div>
      <div class="card">
        <h2>Pays</h2>
        {#if stats.countries?.length}
          <table><thead><tr><th>Pays</th><th class="n">Visiteurs</th><th class="n">Visites</th></tr></thead>
            <tbody>{#each stats.countries as c}<tr><td>{label(c.country)}</td><td class="n">{fmt(c.visitors)}</td><td class="n">{fmt(c.hits)}</td></tr>{/each}</tbody></table>
        {:else}<p class="none">{stats.countries ? 'Aucune donnée.' : DASH}</p>{/if}
      </div>
      <div class="card">
        <h2>Régions</h2>
        {#if stats.regions?.length}
          <table><thead><tr><th>Pays</th><th>Région</th><th class="n">Visiteurs</th><th class="n">Visites</th></tr></thead>
            <tbody>{#each stats.regions.slice(0, 30) as r}<tr><td>{label(r.country)}</td><td>{label(r.region)}</td><td class="n">{fmt(r.visitors)}</td><td class="n">{fmt(r.hits)}</td></tr>{/each}</tbody></table>
        {:else}<p class="none">{stats.regions ? 'Aucune donnée.' : DASH}</p>{/if}
      </div>
      <div class="card">
        <h2>Sites d’origine</h2>
        {#if stats.referrers?.length}
          <table><tbody>{#each stats.referrers as r}<tr><td>{r.ref}</td><td class="n">{fmt(r.hits)}</td></tr>{/each}</tbody></table>
        {:else}<p class="none">{stats.referrers ? 'Aucune donnée.' : DASH}</p>{/if}
      </div>
    </section>
  {:else if loading}
    <p class="meta">Chargement (requêtes CloudWatch Logs Insights, jusqu’à 20 s)…</p>
  {/if}
</main>

<style>
  main { max-width: 1400px; margin: 0 auto; padding: 16px 24px 48px; }
  header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; }
  h1 { font-family: var(--font-head); font-size: 1.5rem; margin: 0; }
  h2 { font-size: 1rem; margin: 0 0 8px; }
  h2 small { font-weight: 400; color: var(--muted); }
  h3 { font-size: 0.95rem; margin: 12px 0 6px; }
  nav { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  nav button[aria-pressed='true'] { border-color: var(--accent); color: var(--accent-text); }
  nav input { min-height: 44px; padding: 6px 8px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); color: var(--text); }
  form { display: flex; flex-direction: column; gap: 8px; max-width: 360px; margin: 48px auto; }
  form input { min-height: 44px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 10px; background: var(--surface); color: var(--text); }
  .error { color: var(--danger); font-weight: 700; }
  .meta, .none { color: var(--muted); }
  .tiles { display: grid; grid-template-columns: repeat(7, 1fr); gap: 12px; margin: 12px 0 16px; }
  .rates { display: grid; grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); gap: 16px 24px; }
  caption { text-align: left; font-weight: 700; padding: 4px 0; }
  .tiles div { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 12px; display: flex; flex-direction: column; }
  .tiles span { color: var(--muted); font-size: 0.9rem; }
  .tiles b { font-size: 1.6rem; font-family: var(--font-head); }
  .tiles small { color: var(--muted); }
  .tiles.usage { grid-template-columns: repeat(4, 1fr); }
  .scroll { overflow-x: auto; }
  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
  .card { background: var(--lvl-3); border: 1px solid var(--border); border-radius: 12px; padding: 12px; min-width: 0; }
  .card.wide { grid-column: 1 / -1; }
  table { width: 100%; border-collapse: collapse; font-size: 0.92rem; }
  th, td { text-align: left; padding: 4px 6px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .n { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .bars .bar { width: 45%; }
  .bars .bar span { display: block; height: 12px; border-radius: 3px; background: var(--accent); }
  @media (max-width: 1023px) {
    main { padding: 12px; }
    .tiles, .tiles.usage { grid-template-columns: repeat(2, 1fr); }
    .grid { grid-template-columns: 1fr; }
  }
</style>
