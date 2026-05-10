// ── SUMMARY NODE ─────────────────────────────────────────────────────────────
function SummaryNode({ data }) {
  return h('div', { className: 'sn' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '68%' }),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    h('div', { className: 'sn-icon' }, data.icon),
    h('div', { className: 'sn-body' },
      h('div', { className: 'sn-label' }, data.label),
      h('div', { className: 'sn-value' }, data.value ?? '—'),
      h('div', { className: 'sn-sub'   }, data.sub   ?? ' ')
    ),
    h('div', { className: `sn-dot ${data.dotClass ?? ''}` })
  );
}

// ── BMS NODE ──────────────────────────────────────────────────────────────────
function BmsNode({ data }) {
  const live  = data.live;
  const soc   = live?.Soc   ?? null;
  const cur   = live?.Dc?.Current ?? null;
  const pwr   = live?.Dc?.Power   ?? null;
  const volt  = live?.Dc?.Voltage ?? null;
  const tmax  = live?.System?.MaxCellTemperature ?? null;
  const alarm = live?.Alarms ? Object.values(live.Alarms).some(v => v > 0) : false;

  return h('div', { className: `bms-nd${alarm ? ' alarm' : ''}` },
    mkHandle('target', Position.Top,   'tt'),
    mkHandle('source', Position.Right, 'sr'),
    h('div', { className: 'bms-hdr' },
      h('span', null, '🔋'),
      h('span', { className: 'bms-name' }, data.label),
      tmax != null ? h('span', { className: 'bms-temp' }, `🌡 ${tmax.toFixed(1)}°C`) : null,
      h('div',  { className: `bms-live-dot${live ? ' on' : ''}` })
    ),
    h('div', { className: 'bms-bar-bg' },
      h('div', {
        className: 'bms-bar-fill',
        style: { width: `${soc ?? 0}%`, background: soc != null ? socColor(soc) : '#e2e8f0' }
      })
    ),
    h('div', { className: 'bms-kpis' },
      h('div', { className: 'bms-kpi' },
        h('div', { className: 'bms-kpi-v' }, soc != null ? `${soc.toFixed(0)}%` : '—'),
        h('div', { className: 'bms-kpi-l' }, 'SOC')
      ),
      h('div', { className: 'bms-kpi' },
        h('div', { className: 'bms-kpi-v' }, volt != null ? `${volt.toFixed(2)}V` : '—'),
        h('div', { className: 'bms-kpi-l' }, 'Tension')
      ),
      h('div', { className: 'bms-kpi' },
        h('div', {
          className: 'bms-kpi-v',
          style: { color: cur != null ? (cur < 0 ? '#dc2626' : cur > 0 ? '#2563eb' : '#0f172a') : undefined }
        }, cur != null ? `${cur > 0 ? '+' : ''}${cur.toFixed(1)}A` : '—'),
        h('div', { className: 'bms-kpi-l' }, 'Courant')
      ),
      h('div', { className: 'bms-kpi' },
        h('div', { className: 'bms-kpi-v' }, pwr != null ? `${pwr > 0 ? '+' : ''}${(pwr / 1000).toFixed(0)}` : '—'),
        h('div', { className: 'bms-kpi-l' }, 'kW')
      )
    )
  );
}

// ── ET112 CARD NODE ───────────────────────────────────────────────────────────
function Et112CardNode({ data }) {
  const live = data.live;
  const connected = live?.connected ?? false;
  const pwr  = live?.power_w ?? null;
  const volt = live?.voltage_v ?? null;
  const cur  = live?.current_a ?? null;
  const imp  = live?.energy_import_wh != null ? live.energy_import_wh / 1000 : null;
  const expW = live?.energy_export_wh != null ? live.energy_export_wh / 1000 : null;
  const addr = data.address ?? 7;
  const icon = data.icon || '☀';

  return h('div', { className: 'et112-card-wrapper' },
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Left,   'sl', { top: '50%' }),
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '50%' }),
    h('div', { className: `bms-card-in-viz ${connected ? '' : 'offline-card'}`, style: { opacity: connected ? 1 : 0.7 } },
      h('div', { className: `bms-hdr ${connected ? '' : 'offline'}` },
        h('div', { className: 'bms-hdr-left' },
          connected ? h('div', { className: 'bms-live' }, h('div', { className: 'live-dot' }), 'LIVE') : h('span', { style: { fontSize: '0.62rem', color: 'var(--muted2)' } }, 'Hors ligne'),
          h('span', { className: 'bms-hdr-name' }, `${icon} `, data.label)
        ),
        h('div', { className: 'bms-hdr-right' }, h('span', { className: 'bms-badge' }, `0x0${addr.toString(16).toUpperCase()}`))
      ),
      connected && live ? h('div', { className: 'kpi4' },
        h('div', { className: 'kpi4-cell t-yellow' },
          h('div', { className: 'kpi4-lbl' }, 'Puissance'),
          h('div', { className: `kpi4-val ${(pwr ?? 0) >= 0 ? 'chg' : 'dch'}` }, pwr != null ? `${pwr.toFixed(0)} W` : '—')
        ),
        h('div', { className: 'kpi4-cell t-blue' },
          h('div', { className: 'kpi4-lbl' }, 'Tension'),
          h('div', { className: 'kpi4-val' }, volt != null ? `${volt.toFixed(1)} V` : '—')
        ),
        h('div', { className: 'kpi4-cell t-orange' },
          h('div', { className: 'kpi4-lbl' }, 'Courant'),
          h('div', { className: 'kpi4-val' }, cur != null ? `${cur.toFixed(2)} A` : '—')
        )
      ) : h('div', { className: 'empty-state' },
        h('div', { className: 'empty-icon' }, '⏳'),
        h('div', { className: 'empty-title' }, 'En attente de données')
      ),
      connected && live ? h('div', { className: 'istrip' },
        h('div', { className: 'icell' },
          h('span', { className: 'i-lbl' }, '⬇ Importée'),
          h('span', { className: 'i-val ok' }, imp != null ? `${imp.toFixed(2)} kWh` : '—')
        ),
        h('div', { className: 'icell' },
          h('span', { className: 'i-lbl' }, '⬆ Exportée'),
          h('span', { className: 'i-val blue' }, expW != null ? `${expW.toFixed(2)} kWh` : '—')
        )
      ) : null
    )
  );
}

// ── DEVICE NODE ───────────────────────────────────────────────────────────────
function DeviceNode({ data }) {
  const live = data.live;
  const pwr  = live?.power_w;
  const nrj  = live?.energy_import_kwh;
  return h('div', { className: 'dev-nd' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('source', Position.Left,   'sl', { top: '65%' }),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    h('div', { className: 'dev-hdr' },
      h('span', null, data.icon ?? '⚡'),
      h('span', { className: 'dev-lbl' }, data.label),
      h('div',  { className: `dev-dot${live?.connected ? ' on' : ''}` })
    ),
    live ? h('div', null,
      h('div', { className: 'dev-pwr' }, pwr != null ? `${(pwr / 1000).toFixed(2)} kW` : '—'),
      h('div', { className: 'dev-nrj' }, nrj != null ? `${nrj.toFixed(1)} kWh total` : ' ')
    ) : h('div', { className: 'dev-wait' }, 'En attente…')
  );
}

// ── HUB NODE ──────────────────────────────────────────────────────────────────
function HubNode({ data }) {
  return h('div', { className: 'hub-nd' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '68%' }),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    h('div', { className: 'hub-icon' }, data.icon),
    h('div', { className: 'hub-lbl'  }, data.label),
    data.sub && h('div', { className: 'hub-sub' }, data.sub)
  );
}

// ── PLACEHOLDER NODE ──────────────────────────────────────────────────────────
function PlaceholderNode({ data }) {
  return h('div', { className: 'ph-nd' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '68%' }),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    data.icon && h('span', { className: 'ph-icon' }, data.icon),
    h('div', { className: 'ph-lbl'   }, data.label),
    h('div', { className: 'ph-badge' }, 'À venir')
  );
}
