// ── ATS RÉSEAU NODE ───────────────────────────────────────────────────────────
function AtsReseauNode({ data }) {
  const active    = (data.sw2 || '').includes('Fermé');
  const connected = (parseInt(data.v2a) || 0) > 50;
  const status    = (data.s2a || '').toLowerCase();
  const isOk      = status.includes('normal');
  const vLevel    = Math.min(100, Math.max(0, ((parseInt(data.v2a) || 0) / 400) * 100));

  const handleClick = useCallback((e) => {
    e.stopPropagation();
    e.preventDefault();
    if (typeof data.onForce === 'function' && !data.disabled) data.onForce();
  }, [data.onForce, data.disabled]);

  return h('div', { className: `pro-node${active ? ' active-source' : ''}` },
    mkHandle('source', Position.Right,  'reseau-out'),
    mkHandle('source', Position.Bottom, 'reseau-et112reseau'),
    h('div', { className: 'pn-banner' },
      h('div', { className: 'pn-banner-left' },
        h('div', { className: 'pn-bicon pn-bicon-res' }, '⚡'),
        h('span', null, 'Réseau')
      ),
      h('div', { className: 'pn-banner-center' },
        h('span', { className: 'pn-volt' }, data.v2a || '--', h('span', { className: 'pn-volt-unit' }, ' V')),
        h('span', { className: `pn-status${connected ? ' online' : ' offline'}` },
          h('span', { className: `pn-dot${connected ? '' : ' off'}` }),
          connected ? 'En ligne' : 'Hors ligne'
        )
      ),
      h('span', { className: `pn-badge${isOk ? ' ok' : ' warn'}` }, isOk ? '✓ OK' : '⚠')
    ),
    h('div', { className: 'pn-bar-row' },
      h('div', { className: `pn-bar-fill${isOk ? ' ok' : ' warn'}`, style: { width: `${vLevel}%` } })
    ),
    h('div', { className: 'pn-strip' },
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'SW2'), h('span', { className: `pn-chip-val${active ? ' ok' : ''}` }, data.sw2 || '--')),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'Max'), h('span', { className: 'pn-chip-val' }, data.max2 || '--')),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'Nbr'), h('span', { className: 'pn-chip-val' }, data.cnt2 || 0)),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'T2'),  h('span', { className: 'pn-chip-val' }, data.t2 || '—'))
    ),
    h('div', { className: 'pn-foot' },
      h('button', {
        className: 'pn-btn pn-btn-warn',
        'data-is-interactive': 'true',
        'data-rev': data._rev,
        onClick: handleClick,
        onMouseDown: (e) => { e.stopPropagation(); e.preventDefault(); },
        onTouchStart: (e) => { e.stopPropagation(); e.preventDefault(); },
        disabled: data.disabled,
        style: { pointerEvents: 'auto', cursor: data.disabled ? 'not-allowed' : 'pointer' }
      }, '⚡ ', data.disabled ? '...' : 'ACTIVER')
    )
  );
}

// ── ATS ONDULEUR NODE ─────────────────────────────────────────────────────────
function AtsOnduleurNode({ data }) {
  const active    = (data.sw1 || '').includes('Fermé');
  const connected = (parseInt(data.v1a) || 0) > 50;
  const status    = (data.s1a || '').toLowerCase();
  const isOk      = status.includes('normal');
  const vLevel    = Math.min(100, Math.max(0, ((parseInt(data.v1a) || 0) / 400) * 100));

  const handleClick = useCallback((e) => {
    e.stopPropagation();
    e.preventDefault();
    if (typeof data.onForce === 'function' && !data.disabled) data.onForce();
  }, [data.onForce, data.disabled]);

  return h('div', { className: `pro-node${active ? ' active-source' : ''}` },
    mkHandle('source', Position.Top,    'onduleur-out'),
    mkHandle('target', Position.Right,  'onduleur-right'),
    mkHandle('source', Position.Bottom, 'onduleur-bottom'),
    h('div', { className: 'pn-banner' },
      h('div', { className: 'pn-banner-left' },
        h('div', { className: 'pn-bicon pn-bicon-inv' }, '🔋'),
        h('span', null, 'Onduleur')
      ),
      h('div', { className: 'pn-banner-center' },
        h('span', { className: 'pn-volt' }, data.v1a || '--', h('span', { className: 'pn-volt-unit' }, ' V')),
        h('span', { className: `pn-status${connected ? ' online' : ' offline'}` },
          h('span', { className: `pn-dot${connected ? '' : ' off'}` }),
          connected ? 'En ligne' : 'Hors ligne'
        )
      ),
      h('span', { className: `pn-badge${isOk ? ' ok' : ' warn'}` }, isOk ? '✓ OK' : '⚠')
    ),
    h('div', { className: 'pn-bar-row' },
      h('div', { className: `pn-bar-fill${isOk ? ' ok' : ' warn'}`, style: { width: `${vLevel}%` } })
    ),
    h('div', { className: 'pn-strip' },
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'SW1'), h('span', { className: `pn-chip-val${active ? ' ok' : ''}` }, data.sw1 || '--')),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'Max'), h('span', { className: 'pn-chip-val' }, data.max1 || '--')),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'Nbr'), h('span', { className: 'pn-chip-val' }, data.cnt1 || 0)),
      h('div', { className: 'pn-chip' }, h('span', { className: 'pn-chip-lbl' }, 'T1'),  h('span', { className: 'pn-chip-val' }, data.t1 || '—'))
    ),
    h('div', { className: 'pn-foot' },
      h('button', {
        className: 'pn-btn pn-btn-prim',
        'data-is-interactive': 'true',
        'data-rev': data._rev,
        onClick: handleClick,
        onMouseDown: (e) => { e.stopPropagation(); e.preventDefault(); },
        onTouchStart: (e) => { e.stopPropagation(); e.preventDefault(); },
        disabled: data.disabled,
        style: { pointerEvents: 'auto', cursor: data.disabled ? 'not-allowed' : 'pointer' }
      }, '🔋 ', data.disabled ? '...' : 'ACTIVER')
    )
  );
}

// ── ATS MAIN NODE ─────────────────────────────────────────────────────────────
function AtsMainNode({ data }) {
  const remoteOn  = (data.swRemote || '').includes('Activé');
  const hasFault  = (data.swFault || 'Aucun') !== 'Aucun';
  const centerOff = (data.middleOFF || '').includes('Activé');

  const handleToggleClick = useCallback((e) => {
    e.stopPropagation(); e.preventDefault();
    if (typeof data.onToggleRemote === 'function' && !data.disabled) data.onToggleRemote();
  }, [data.onToggleRemote, data.disabled]);

  const handleForceOffClick = useCallback((e) => {
    e.stopPropagation(); e.preventDefault();
    if (typeof data.onForceOff === 'function' && !data.disabled) {
      if (window.confirm('⚠️ Confirmer Double OFF ?')) data.onForceOff();
    }
  }, [data.onForceOff, data.disabled]);

  return h('div', { className: 'pro-node' },
    mkHandle('target', Position.Left,   'ats-in-reseau'),
    mkHandle('source', Position.Right,  'main-right'),
    mkHandle('target', Position.Bottom, 'ats-in-onduleur', { top: '65%' }),
    h('div', { className: 'pn-banner' },
      h('div', { className: 'pn-banner-left' },
        h('div', { className: 'pn-bicon pn-bicon-ats' }, '🔀'),
        h('span', null, 'ATS')
      ),
      h('div', { className: 'pn-banner-center',
        'data-is-interactive': 'true',
        'data-rev': data._rev,
        style: { cursor: data.disabled ? 'default' : 'pointer', pointerEvents: 'auto' },
        onClick: handleToggleClick,
        onMouseDown: (e) => { e.stopPropagation(); e.preventDefault(); },
        onTouchStart: (e) => { e.stopPropagation(); e.preventDefault(); }
      },
        h('span', { className: 'pn-volt', style: { fontSize: '9px' } }, data.swMode || '—'),
        h('span', { className: `pn-status${remoteOn ? ' online' : ' offline'}` },
          h('span', { className: `pn-dot${remoteOn ? '' : ' off'}` }),
          h('div', { className: `pn-toggle${remoteOn ? ' on' : ''}` })
        )
      ),
      h('span', { className: `pn-badge${hasFault ? ' warn' : ' ok'}` }, hasFault ? '⚠ ERR' : '✓ OK')
    ),
    h('div', { className: 'pn-bar-row' },
      h('div', { className: `pn-bar-fill${hasFault ? ' warn' : ' ok'}`, style: { width: '100%' } })
    ),
    h('div', { className: 'pn-ats-sw' },
      h('div', { className: 'pn-sw-card' }, h('span', { className: 'pn-sw-label' }, 'SW1'), h('span', { className: `pn-sw-val${(data.sw1||'').includes('Fermé') ? ' closed' : ' open'}` }, data.sw1 || 'Ouvert')),
      h('div', { className: 'pn-sw-card' }, h('span', { className: 'pn-sw-label' }, 'SW2'), h('span', { className: `pn-sw-val${(data.sw2||'').includes('Fermé') ? ' closed' : ' open'}` }, data.sw2 || 'Ouvert'))
    ),
    h('div', { className: 'pn-info-combined' },
      h('div', { className: 'pn-info-row' }, h('span', { className: 'pn-info-key' }, 'Mode'),   h('span', { className: 'pn-info-val' }, data.swMode || '—')),
      h('div', { className: 'pn-info-row' }, h('span', { className: 'pn-info-key' }, 'Défaut'), h('span', { className: `pn-info-val${hasFault ? ' err' : ' ok'}` }, hasFault ? '⚠ Erreur' : '✓ Aucun'))
    ),
    h('div', { className: 'pn-info-row' }, h('span', { className: 'pn-info-key' }, 'Source'), h('span', { className: `pn-info-val${centerOff ? ' err' : ''}` }, centerOff ? '⚠ DOUBLE OFF' : data.active_source || '—')),
    h('div', { className: 'pn-foot' },
      h('button', {
        className: 'pn-btn pn-btn-danger',
        'data-is-interactive': 'true',
        'data-rev': data._rev,
        onClick: handleForceOffClick,
        onMouseDown: (e) => { e.stopPropagation(); e.preventDefault(); },
        onTouchStart: (e) => { e.stopPropagation(); e.preventDefault(); },
        disabled: data.disabled,
        style: { pointerEvents: 'auto', cursor: data.disabled ? 'not-allowed' : 'pointer' }
      }, '⏹️ ', data.disabled ? '...' : 'Double OFF')
    )
  );
}

// ── INVERTER NODE ─────────────────────────────────────────────────────────────
function InverterNode({ data }) {
  const inv = data.live;
  if (!inv) return h('div', { className: 'inv-card' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    mkHandle('source', Position.Left,   'sl', { top: '65%' }),
    h('div', { className: 'inv-hdr' },
      h('span', { className: 'inv-hdr-icon' }, '⚡'),
      h('span', { className: 'inv-hdr-title' }, 'EasySolar-II'),
      h('div', { className: 'inv-dot' })
    ),
    h('div', { className: 'inv-wait' }, 'En attente…')
  );

  const acV   = inv.ac_output_voltage_v  ?? null;
  const acI   = inv.ac_output_current_a  ?? null;
  const acP   = inv.ac_output_power_w    ?? null;
  const acF   = inv.ac_out_frequency_hz  ?? null;
  const dcV   = inv.voltage_v            ?? null;
  const dcI   = inv.current_a            ?? null;
  const dcP   = inv.power_w              ?? null;
  const ignAc = inv.ac_in_ignore         ?? null;
  const state = inv.state                ?? '—';

  const stateClass = (() => {
    const l = state.toLowerCase();
    if (l.includes('invert')) return 'inverting';
    if (l.includes('charg') || l.includes('bulk') || l.includes('absorb') || l.includes('float')) return 'charging';
    return 'off';
  })();

  const fmtFreq = (f) => f != null ? `${f.toFixed(2)} Hz` : '— Hz';

  return h('div', { className: 'inv-card' },
    h('div', { className: 'inv-freq-badge' }, fmtFreq(acF)),
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Right,  'tr', { top: '65%' }),
    mkHandle('source', Position.Left,   'sl', { top: '65%' }),
    h('div', { className: 'inv-hdr' },
      h('span', { className: 'inv-hdr-icon' }, '⚡'),
      h('span', { className: 'inv-hdr-title' }, 'EasySolar-II'),
      h('span', { className: `inv-state-badge ${stateClass}` }, state),
      h('div', { className: 'inv-dot live' })
    ),
    h('div', { className: 'inv-ignore-bar' },
      h('span', { className: 'inv-ignore-lbl' }, 'AC In Ignore'),
      h('span', { className: `inv-ignore-val ${ignAc ? 'active' : 'normal'}` }, ignAc ? 'ACTIF' : 'Normal')
    ),
    h('div', { className: 'inv-section-hdr' }, 'AC Out'),
    h('div', { className: 'inv-kpi-row' },
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Tension'),   h('span', { className: 'inv-kpi-val ac' }, acV != null ? `${acV.toFixed(1)}V` : '—')),
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Courant'),   h('span', { className: 'inv-kpi-val ac' }, acI != null ? `${acI.toFixed(1)}A` : '—')),
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Puissance'), h('span', { className: 'inv-kpi-val ac' }, acP != null ? `${acP.toFixed(0)}W` : '—'))
    ),
    h('div', { className: 'inv-section-hdr' }, 'DC'),
    h('div', { className: 'inv-kpi-row' },
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Tension'),   h('span', { className: 'inv-kpi-val dc' }, dcV != null ? `${dcV.toFixed(1)}V` : '—')),
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Courant'),   h('span', { className: 'inv-kpi-val dc' }, dcI != null ? `${dcI.toFixed(1)}A` : '—')),
      h('div', { className: 'inv-kpi' }, h('span', { className: 'inv-kpi-lbl' }, 'Puissance'), h('span', { className: 'inv-kpi-val dc' }, dcP != null ? `${dcP.toFixed(0)}W` : '—'))
    )
  );
}
