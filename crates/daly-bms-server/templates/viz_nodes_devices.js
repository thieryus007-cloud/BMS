// ── TONGOU GROUP NODE ─────────────────────────────────────────────────────────
const TASMOTA_IDS = [1, 2, 3, 4, 5];

const TongouGroupNode = memo(function TongouGroupNode({ data }) {
  const switches = (data.switches && data.switches.length > 0)
    ? data.switches
    : TASMOTA_IDS.map(id => ({ id, name: `Switch ${id}`, connected: false }));

  const disabled = data.disabled || false;

  const handleToggle = useCallback((switchId, currentState) => {
    if (disabled) return;
    const cmd = !currentState ? 'on' : 'off';
    fetch(`/api/v1/tasmota/${switchId}/control`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ state: cmd }),
      cache: 'no-cache'
    })
    .then(r => r.json())
    .then(d => console.log(`[TONGOU] OK ID ${switchId}:`, d))
    .catch(err => console.error(`[TONGOU] Erreur ID ${switchId}:`, err));
  }, [disabled]);

  const countOnline = switches.filter(s => s.connected).length;

  return h('div', { className: 'tongou-group-card' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,  'tl', { top: '20%' }),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '68%' }),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    h('div', { className: 'tg-header' },
      h('span', { className: 'tg-header-icon' }, '🔌'),
      h('span', { className: 'tg-header-title' }, 'Meter Switchs'),
      h('span', { className: 'tg-header-count' }, `${countOnline}/${switches.length} en ligne`)
    ),
    h('div', { className: 'tg-switch-list' },
      switches.map((sw, idx) => {
        const switchId  = sw.id || idx + 1;
        const connected = sw.connected ?? false;
        const powerOn   = sw.power_on ?? false;
        const powerW    = sw.power_w ?? null;
        const voltage   = sw.voltage_v ?? null;
        const current   = sw.current_a ?? null;
        const energy    = sw.energy_today_kwh ?? null;
        const name      = sw.name || `Switch ${switchId}`;

        return h('div', { key: switchId, className: 'tg-switch-item' },
          h('div', { className: 'tg-row1' },
            h('span', { className: 'tg-name' }, name),
            h('span', { className: 'tg-id' }, `ID: ${switchId}`),
            h('button', {
              className: `tg-toggle-btn ${powerOn ? 'off' : ''}`,
              'data-is-interactive': 'true',
              disabled: disabled || !connected,
              onClick: (e) => { e.stopPropagation(); handleToggle(switchId, powerOn); }
            }, disabled ? '...' : (connected ? (powerOn ? 'Éteindre' : 'Allumer') : 'Hors ligne'))
          ),
          h('div', { className: `tg-row2 ${connected ? (powerOn ? 'on' : 'off') : ''}` },
            h('div', { className: 'tg-metric' },
              h('span', { className: 'tg-metric-label' }, 'Puissance'),
              h('span', { className: 'tg-metric-value' }, powerW != null ? `${powerW.toFixed(0)} W` : '—')
            ),
            h('div', { className: 'tg-metric' },
              h('span', { className: 'tg-metric-label' }, 'Tension'),
              h('span', { className: 'tg-metric-value' }, voltage != null ? `${voltage.toFixed(1)} V` : '—')
            ),
            h('div', { className: 'tg-metric' },
              h('span', { className: 'tg-metric-label' }, 'Courant'),
              h('span', { className: 'tg-metric-value' }, current != null ? `${current.toFixed(2)} A` : '—')
            ),
            h('div', { className: 'tg-metric' },
              h('span', { className: 'tg-metric-label' }, "Aujourd'hui"),
              h('span', { className: 'tg-metric-value small' }, energy != null ? `${energy.toFixed(2)} kWh` : '—')
            )
          )
        );
      })
    )
  );
});

// ── WATERHEATER / CLIMATISATION NODE ─────────────────────────────────────────
function WaterHeaterNode({ data }) {
  const et    = data.live;
  const hp    = data.heatpump;
  const isClim = data.isClimatisation ?? false;

  const connected = et?.connected ?? false;
  const pwr       = et?.power_w   ?? null;
  const nrj       = et?.energy_import_wh != null ? et.energy_import_wh / 1000 : null;

  const hpState  = hp?.state               ?? null;
  const currTemp = hp?.temperature         ?? null;
  const targTemp = hp?.target_temperature  ?? null;
  const hpLive   = hp?.connected           ?? false;

  const modeLabel = hpState === 0 ? 'Vacances' : hpState === 1 ? 'HP Normal' : hpState === 2 ? 'Turbo' : '—';
  const modeCls   = hpState === 0 ? 'vacances' : hpState === 1 ? 'hp' : hpState === 2 ? 'turbo' : 'unknown';

  return h('div', { className: 'waterheater-card' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    mkHandle('source', Position.Left,   'sl', { top: '65%' }),
    h('div', { className: `wh-header${isClim ? ' clim-header' : ''}` },
      h('span', { className: 'wh-header-icon' }, data.icon ?? (isClim ? '❄️' : '🚿')),
      h('span', { className: 'wh-header-title' }, data.label),
      h('div',  { className: `wh-header-dot${(connected || hpLive) ? ' live' : ''}` })
    ),
    h('div', { className: 'wh-body' },
      h('div', { className: 'wh-mode-row' },
        h('span', { className: `wh-mode-badge ${modeCls}` }, modeLabel),
        h('span', { style: { fontSize: '0.62rem', color: '#64748b' } }, hpLive ? '● Venus' : '○ Venus')
      ),
      h('div', { className: 'wh-temps-row' },
        h('div', { className: 'wh-temp-cell' },
          h('span', { className: 'wh-temp-lbl' }, 'Temp. actuelle'),
          h('span', { className: 'wh-temp-val' }, currTemp != null ? `${currTemp.toFixed(1)}°C` : '—')
        ),
        h('div', { className: 'wh-temp-cell' },
          h('span', { className: 'wh-temp-lbl' }, 'Temp. cible'),
          h('span', { className: 'wh-temp-val' }, targTemp != null ? `${targTemp.toFixed(1)}°C` : '—')
        )
      ),
      connected && et
        ? h('div', { className: 'wh-et112-row' },
            h('div', { className: 'wh-et112-metric' },
              h('span', { className: 'wh-et112-lbl' }, 'Puissance'),
              h('span', { className: 'wh-et112-val' }, pwr != null ? `${pwr.toFixed(0)} W` : '—')
            ),
            h('div', { className: 'wh-et112-metric' },
              h('span', { className: 'wh-et112-lbl' }, 'Énergie'),
              h('span', { className: 'wh-et112-val' }, nrj != null ? `${nrj.toFixed(2)} kWh` : '—')
            )
          )
        : h('div', { className: 'wh-wait' }, 'ET112 en attente…')
    )
  );
}
