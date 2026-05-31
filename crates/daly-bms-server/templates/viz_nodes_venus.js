// ── MPPT GROUP NODE ───────────────────────────────────────────────────────────
// Affiche toujours MPPT-273 et MPPT-289 (instances connues en production).
// Si les données individuelles sont indisponibles (format v1 agrégé), les lignes
// restent visibles avec "—" pour les métriques.
const MPPT_KNOWN_INSTANCES = [273, 289];

function MPPTGroupNode({ data }) {
  const mppts       = data.mppts       ?? [];
  const totalPowerW = data.totalPowerW ?? 0;
  const totalDcA    = data.totalDcA    ?? 0;

  const stateClass = (s) => {
    if (!s) return '';
    const l = s.toLowerCase();
    if (l === 'off')              return 'off';
    if (l === 'fault')            return 'fault';
    if (l === 'bulk')             return 'bulk';
    if (l.includes('absorb'))     return 'bulk';
    if (l.includes('float') || l.includes('storage')) return 'float';
    return '';
  };

  // Si les données individuelles des instances connues sont présentes, les afficher.
  // Sinon, toujours montrer les deux lignes connues avec placeholders.
  const mpptById = {};
  for (const m of mppts) { mpptById[m.instance] = m; }
  const hasKnown = MPPT_KNOWN_INSTANCES.some(inst => mpptById[inst]);
  const displayList = hasKnown
    ? MPPT_KNOWN_INSTANCES.map(inst => mpptById[inst] || { instance: inst, state: null, pv_voltage_v: null, dc_current_a: null, power_w: null, yield_today_kwh: null })
    : mppts.length > 0 ? mppts : MPPT_KNOWN_INSTANCES.map(inst => ({ instance: inst, state: null, pv_voltage_v: null, dc_current_a: null, power_w: null, yield_today_kwh: null }));

  return h('div', { className: 'mppt-group-card' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    mkHandle('source', Position.Left,   'sl', { top: '10%' }),
    h('div', { className: 'mg-header' },
      h('span', { className: 'mg-header-icon' }, '☀️'),
      h('span', { className: 'mg-header-title' }, 'SmartSolar MPPT'),
      h('span', { style: { fontSize:'0.65rem', opacity:0.8, marginLeft:'auto' } }, `${MPPT_KNOWN_INSTANCES.length} chargeurs`)
    ),
    h('div', { className: 'mg-totals' },
      h('div', { className: 'mg-total-chip' },
        h('span', { className: 'mg-total-lbl' }, 'Total Puissance'),
        h('span', { className: 'mg-total-val' }, `${Math.round(totalPowerW)} W`)
      ),
      h('div', { className: 'mg-total-chip' },
        h('span', { className: 'mg-total-lbl' }, 'Total DC'),
        h('span', { className: 'mg-total-val' }, `${Math.round(totalDcA)} A`)
      )
    ),
    h('div', { className: 'mg-mppt-list' },
      displayList.map(function(m) {
        const inst = m.instance;
        const name = `MPPT-${inst}`;
        const s    = m.state ?? null;
        const pvV  = m.pv_voltage_v ?? null;
        const dcA  = m.dc_current_a ?? null;
        const pw   = m.power_w ?? null;
        const yld  = m.yield_today_kwh ?? null;
        const hasData = pvV != null || pw != null;
        return h('div', { key: inst, className: 'mg-mppt-row' },
          h('span', { className: 'mg-mppt-name' }, name),
          h('span', { className: `mg-mppt-state ${stateClass(s)}` }, s ?? (hasData ? 'Actif' : '—')),
          h('div', { className: 'mg-mppt-metrics' },
            h('div', { className: 'mg-metric' },
              h('span', { className: 'mg-metric-lbl' }, 'PV'),
              h('span', { className: 'mg-metric-val' }, pvV != null ? `${pvV.toFixed(1)}V` : '—')
            ),
            h('div', { className: 'mg-metric' },
              h('span', { className: 'mg-metric-lbl' }, 'DC'),
              h('span', { className: 'mg-metric-val' }, dcA != null ? `${dcA.toFixed(1)}A` : '—')
            ),
            h('div', { className: 'mg-metric' },
              h('span', { className: 'mg-metric-lbl' }, 'W'),
              h('span', { className: 'mg-metric-val' }, pw != null ? `${Math.round(pw)}` : '—')
            ),
            h('div', { className: 'mg-metric' },
              h('span', { className: 'mg-metric-lbl' }, 'kWh'),
              h('span', { className: 'mg-metric-val' }, yld != null ? yld.toFixed(2) : '—')
            )
          )
        );
      })
    )
  );
}

// ── SMARTSHUNT NODE ───────────────────────────────────────────────────────────
function SmartShuntNode({ data }) {
  const live    = data.live;
  const soc     = live?.soc_percent    ?? null;
  const voltage = live?.voltage_v      ?? null;
  const current = live?.current_a      ?? null;
  const power   = live?.power_w        ?? null;
  const state   = live?.state          ?? null;
  const ttgMin  = live?.time_to_go_min ?? null;
  // Ah chargés / déchargés aujourd'hui : valeurs fournies directement par
  // l'energy-manager (intégration courant×temps depuis minuit). Règle 13 —
  // on privilégie cette mesure ; le recalcul PromQL ci-dessous (vmCharge /
  // vmDischarge) ne sert que de repli si ces champs sont absents.
  const ahChaLive = live?.ah_charged_today    ?? null;
  const ahDisLive = live?.ah_discharged_today ?? null;

  const [vmCharge,    setVmCharge]    = useState(null);
  const [vmDischarge, setVmDischarge] = useState(null);

  useEffect(function() {
    async function fetchDailyTotals() {
      const now = new Date();
      const h = Math.max(1, Math.round(now.getHours() + now.getMinutes() / 60));
      const win = h + 'h';
      const qDis = '-avg_over_time(clamp_max(venus_shunt_current_a,0)[' + win + '])*' + h;
      const qCha = '-avg_over_time(clamp_min(venus_shunt_current_a,0)[' + win + '])*' + h;
      try {
        const [rDis, rCha] = await Promise.all([
          fetch('/api/v1/query?query=' + encodeURIComponent(qDis)).then(r => r.ok ? r.json() : null),
          fetch('/api/v1/query?query=' + encodeURIComponent(qCha)).then(r => r.ok ? r.json() : null),
        ]);
        const parseVm = (resp) => {
          if (!resp || resp.status !== 'success') return null;
          const v = resp.data?.result?.[0]?.value?.[1];
          if (v == null) return null;
          const n = parseFloat(v);
          return isNaN(n) ? null : n;
        };
        const dis = parseVm(rDis);
        const cha = parseVm(rCha);
        setVmDischarge(dis != null ? Math.abs(dis) : null);
        setVmCharge(cha != null ? Math.abs(cha) : null);
      } catch (_) {}
    }
    fetchDailyTotals();
    const t = setInterval(fetchDailyTotals, 60000);
    return () => clearInterval(t);
  }, []);

  // Valeur affichée : energy-manager en priorité, repli sur le recalcul PromQL.
  const chargeAh    = ahChaLive != null ? Math.abs(ahChaLive) : vmCharge;
  const dischargeAh = ahDisLive != null ? Math.abs(ahDisLive) : vmDischarge;

  const stateClass = state === 'Charging' ? 'charging' : state === 'Discharging' ? 'discharging' : 'idle';

  const fmtTtg = (min) => {
    if (min == null) return '—';
    if (min >= 1440) return `${(min/1440).toFixed(0)}j`;
    if (min >= 60)   return `${Math.floor(min/60)}h${Math.round(min%60).toString().padStart(2,'0')}`;
    return `${Math.round(min)} min`;
  };

  return h('div', { className: 'ss-card' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    mkHandle('source', Position.Left,   'sl', { top: '65%' }),
    h('div', { className: 'ss-hdr' },
      h('span', { className: 'ss-hdr-icon' }, '⚡'),
      h('span', { className: 'ss-hdr-title' }, 'SmartShunt'),
      state && h('span', { className: `ss-state-badge ${stateClass}` }, state),
      h('div', { className: `ss-dot${live ? ' live' : ''}` })
    ),
    h('div', { className: 'ss-bar-bg' },
      h('div', { className: 'ss-bar-fill', style: { width: `${soc ?? 0}%`, background: soc != null ? socColor(soc) : '#e2e8f0' } })
    ),
    live ? h('div', null,
      h('div', { className: 'ss-grid' },
        h('div', { className: 'ss-cell' },
          h('span', { className: 'ss-cell-lbl' }, 'SOC'),
          h('span', { className: 'ss-cell-val', style: { color: soc != null ? socColor(soc) : undefined } }, soc != null ? `${soc.toFixed(1)}%` : '—')
        ),
        h('div', { className: 'ss-cell' },
          h('span', { className: 'ss-cell-lbl' }, 'Tension'),
          h('span', { className: 'ss-cell-val' }, voltage != null ? `${voltage.toFixed(1)}V` : '—')
        ),
        h('div', { className: 'ss-cell' },
          h('span', { className: 'ss-cell-lbl' }, 'Courant'),
          h('span', { className: `ss-cell-val ${current != null ? (current < 0 ? 'neg' : current > 0.1 ? 'pos' : '') : ''}` },
            current != null ? `${current > 0 ? '+' : ''}${current.toFixed(1)}A` : '—')
        )
      ),
      h('div', { className: 'ss-row' },
        h('span', { className: 'ss-row-lbl' }, 'Puissance'),
        h('span', { className: 'ss-row-val' }, power != null ? `${power > 0 ? '+' : ''}${power.toFixed(0)} W` : '—')
      ),
      h('div', { style: { display: 'flex', gap: '1rem', flexWrap: 'wrap' } },
        h('div', { className: 'ss-row', style: { flex: '1 1 0', minWidth: 0 } },
          h('span', { className: 'ss-row-lbl' }, 'Temps restant'),
          h('span', { className: 'ss-row-val ttg' }, fmtTtg(ttgMin))
        ),
        h('div', { className: 'ss-row', style: { flex: '1 1 0', minWidth: 0 } },
          h('span', { className: 'ss-row-lbl' }, '⬆ Chargée (auj.)'),
          h('span', { className: 'ss-row-val pos' }, chargeAh    != null ? `${chargeAh.toFixed(1)} Ah`    : '—')
        ),
        h('div', { className: 'ss-row', style: { flex: '1 1 0', minWidth: 0 } },
          h('span', { className: 'ss-row-lbl' }, '⬇ Déchargée (auj.)'),
          h('span', { className: 'ss-row-val neg' }, dischargeAh != null ? `${dischargeAh.toFixed(1)} Ah` : '—')
        )
      )
    ) : h('div', { className: 'ss-wait' }, 'En attente…')
  );
}

// ── TEMPERATURE NODE ──────────────────────────────────────────────────────────
function TemperatureNode({ data }) {
  const live     = data.live;
  const temp     = live?.temp_c          ?? null;
  const humidity = live?.humidity_percent ?? null;
  const tempStr  = temp     != null ? `${temp.toFixed(1)}°C`       : '—';
  const humStr   = humidity != null ? `${humidity.toFixed(0)}% HR` : null;

  return h('div', { className: 'sn', style: { minWidth: '260px' } },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '68%' }),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    h('div', { className: 'sn-icon' }, data.icon ?? '🌡️'),
    h('div', { className: 'sn-body' },
      h('div', { className: 'sn-label' }, data.label),
      h('div', { className: 'sn-value', style: { display: 'flex', alignItems: 'baseline', gap: '1.2rem' } },
        h('span', null, tempStr),
        humStr != null ? h('span', null, humStr) : null
      ),
      h('div', { className: 'sn-sub'   }, live ? 'Ext.' : 'En attente…')
    ),
    h('div', { className: `sn-dot${live ? ' live' : ''}` })
  );
}
