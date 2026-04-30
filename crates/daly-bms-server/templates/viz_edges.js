// ── EDGE CHART OVERLAY (hover → mini ECharts popup 6h depuis VictoriaMetrics) ────
function EdgeChartOverlay({ labelX, labelY, history, color }) {
  const [hover, setHover]     = useState(false);
  const [payload, setPayload] = useState(null);
  const [loading, setLoading] = useState(false);
  const chartDivRef = useRef(null);
  const chartRef    = useRef(null);
  const hideTimer   = useRef(null);

  const metric = history?.metric;
  const bmsId  = history?.bms_id;
  const addr   = history?.address;

  function scheduleShow() {
    if (hideTimer.current) { clearTimeout(hideTimer.current); hideTimer.current = null; }
    setHover(true);
  }
  function scheduleHide() {
    if (hideTimer.current) clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => setHover(false), 120);
  }

  useEffect(function () {
    if (!hover || !metric) return;
    let cancelled = false;
    setLoading(true);
    let url = '/api/v1/chart/edge-history?metric=' + encodeURIComponent(metric) + '&minutes=360';
    if (bmsId) url += '&bms_id=' + encodeURIComponent(bmsId);
    if (addr)  url += '&address=' + encodeURIComponent(addr);
    fetch(url)
      .then(r => r.json())
      .then(j => { if (!cancelled) { setPayload(j); setLoading(false); } })
      .catch(err => {
        console.warn('[EdgeChartOverlay] fetch error:', err);
        if (!cancelled) { setLoading(false); setPayload({ ok: false, series: [] }); }
      });
    return function () { cancelled = true; };
  }, [hover, metric, bmsId, addr]);

  useEffect(function () {
    if (!hover) return;
    if (!payload || !payload.ok) return;
    if (!chartDivRef.current || !window.echarts) return;
    if (!chartRef.current) chartRef.current = window.echarts.init(chartDivRef.current);
    const series = payload.series || [];
    const times  = series.map(p => p.t);
    const vals   = series.map(p => p.v);
    const hasNeg = vals.some(v => v < 0);
    const hasPos = vals.some(v => v > 0);
    const option = {
      animation: false,
      grid: { top: 24, right: 10, bottom: 24, left: 40 },
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'line' },
        formatter: function (p) {
          if (!p || !p.length) return '';
          const a = p[0];
          const v = typeof a.value === 'number' ? a.value : 0;
          return a.axisValue + '<br/><b>' + v.toFixed(2) + ' ' + (payload.unit || '') + '</b>';
        },
      },
      xAxis: {
        type: 'category', data: times, boundaryGap: false,
        axisLabel: { fontSize: 9, color: '#64748b' },
        axisLine:  { lineStyle: { color: '#cbd5e1' } },
      },
      yAxis: {
        type: 'value', name: payload.unit || '',
        nameTextStyle: { fontSize: 9, color: '#64748b' },
        axisLabel:     { fontSize: 9, color: '#64748b' },
        axisLine:      { lineStyle: { color: '#cbd5e1' } },
        splitLine:     { lineStyle: { color: '#e2e8f0' } },
      },
      series: [{
        type: 'line', smooth: true, showSymbol: false, data: vals, sampling: 'lttb',
        lineStyle: { color, width: 2 },
        areaStyle: hasNeg && hasPos
          ? { origin: 0, color: { type: 'linear', x: 0, y: 0, x2: 0, y2: 1,
              colorStops: [
                { offset: 0,   color: 'rgba(34,197,94,0.35)' },
                { offset: 0.5, color: 'rgba(34,197,94,0)'    },
                { offset: 0.5, color: 'rgba(239,68,68,0)'    },
                { offset: 1,   color: 'rgba(239,68,68,0.35)' },
              ] } }
          : { color, opacity: 0.22 },
        markLine: { silent: true, symbol: 'none', label: { show: false },
          lineStyle: { color: '#94a3b8', type: 'dashed', width: 1 }, data: [{ yAxis: 0 }] },
      }],
    };
    chartRef.current.setOption(option, true);
    chartRef.current.resize();
  }, [hover, payload, color]);

  useEffect(function () {
    if (hover) return;
    if (chartRef.current) { try { chartRef.current.dispose(); } catch (_) {} chartRef.current = null; }
  }, [hover]);

  useEffect(function () {
    return function () {
      if (chartRef.current) { try { chartRef.current.dispose(); } catch (_) {} chartRef.current = null; }
      if (hideTimer.current) { clearTimeout(hideTimer.current); hideTimer.current = null; }
    };
  }, []);

  if (!history || !EdgeLabelRenderer) return null;

  const triggerStyle = {
    position: 'absolute', transform: `translate(-50%,-50%) translate(${labelX}px,${labelY}px)`,
    width: 22, height: 22, borderRadius: '50%',
    background: hover ? color : '#ffffff', border: `2px solid ${color}`,
    color: hover ? '#fff' : color, pointerEvents: 'all', cursor: 'pointer',
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    fontSize: 11, fontWeight: 700, boxShadow: '0 2px 5px rgba(0,0,0,0.15)',
    transition: 'background 0.15s, color 0.15s', zIndex: 9, userSelect: 'none',
  };
  const popupStyle = {
    position: 'absolute', transform: `translate(-50%,16px) translate(${labelX}px,${labelY}px)`,
    width: 415, height: 260, background: '#ffffff', borderRadius: 10,
    boxShadow: '0 8px 24px rgba(0,0,0,0.22)', border: `1px solid ${color}`,
    pointerEvents: 'all', zIndex: 20, padding: '6px 8px 8px 8px',
    display: 'flex', flexDirection: 'column', gap: 4,
  };
  const headerStyle = {
    fontSize: 11, fontWeight: 600, color: '#334155',
    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
  };

  const hoverElements = [];
  if (hover) {
    hoverElements.push(h('div', { key: 'popup', style: popupStyle, onMouseEnter: scheduleShow, onMouseLeave: scheduleHide },
      h('div', { style: headerStyle },
        h('span', null, history.label || 'Courant — 6h'),
        h('span', { style: { color: '#94a3b8', fontWeight: 400 } }, 'VM 6h')
      ),
      h('div', { ref: chartDivRef, style: { flex: 1, width: '100%', minHeight: 0 } }),
      loading && h('div', { style: { position:'absolute', inset:0, display:'flex', alignItems:'center', justifyContent:'center', color:'#64748b', fontSize:11 } }, 'Chargement…'),
      (!loading && payload && (!payload.ok || !payload.series || payload.series.length === 0)) &&
        h('div', { style: { position:'absolute', inset:0, display:'flex', alignItems:'center', justifyContent:'center', color:'#94a3b8', fontSize:11 } }, 'Pas de données sur 6 h')
    ));
  }

  return h(EdgeLabelRenderer, null,
    h('div', { style: triggerStyle, onMouseEnter: scheduleShow, onMouseLeave: scheduleHide, title: 'Historique 6 h' }, '📈'),
    ...hoverElements,
  );
}

// ── ANIMATED FLOW EDGE (WAAPI, chemin smooth-step) ────────────────────────────
function AnimatedFlowEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, style, data }) {
  const color      = style?.stroke ?? '#2563eb';
  const pathResult = useMemo(
    () => getSmoothStepPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition, borderRadius: 5 }),
    [sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition]
  );
  const edgePath = pathResult[0];
  const labelX   = pathResult[1];
  const labelY   = pathResult[2];
  const isH      = Math.abs(targetX - sourceX) >= Math.abs(targetY - sourceY);
  const ox = isH ? 0 : 3;
  const oy = isH ? 3 : 0;

  const flowValue  = data?.flowValue    ?? 0;
  const flowSpeed  = data?.flowSpeed    ?? 1;
  const flowColor  = data?.flowColor    ?? color;
  const reverse    = data?.reverse      ?? false;
  const pCount     = Math.max(1, Math.min(5, data?.particleCount ?? 2));
  const isActive   = flowValue > 0;

  const particleRefs = useRef([]);
  const animsRef     = useRef([]);

  useEffect(function () {
    animsRef.current.forEach(function (a) { try { if (a) a.cancel(); } catch (_) {} });
    animsRef.current = [];
    if (!isActive) return;
    const duration  = Math.max(500, Math.min(5000, 2000 / Math.max(0.4, flowSpeed)));
    const keyframes = reverse
      ? [{ offsetDistance: '100%' }, { offsetDistance: '0%'   }]
      : [{ offsetDistance: '0%'   }, { offsetDistance: '100%' }];
    const newAnims = [];
    for (let i = 0; i < pCount; i++) {
      const el = particleRefs.current[i];
      if (!el) continue;
      try {
        el.style.offsetPath = "path('" + edgePath + "')";
        newAnims.push(el.animate(keyframes, {
          duration, iterations: Infinity, easing: 'linear',
          delay: -(duration / pCount) * i,
        }));
      } catch (err) { console.warn('[AnimatedFlowEdge] WAAPI:', err); }
    }
    animsRef.current = newAnims;
    return function () { newAnims.forEach(function (a) { try { if (a) a.cancel(); } catch (_) {} }); };
  }, [edgePath, isActive, flowSpeed, reverse, pCount]);

  const lineOpacity = isActive ? 0.55 : 0.25;
  const lineStyle   = { fill: 'none', stroke: color, strokeWidth: 2, strokeOpacity: lineOpacity, strokeLinecap: 'round' };
  const children    = [
    h('path', { key: 'l1', d: edgePath, style: lineStyle, transform: 'translate(' + ox    + ',' + oy    + ')' }),
    h('path', { key: 'l2', d: edgePath, style: lineStyle, transform: 'translate(' + (-ox) + ',' + (-oy) + ')' }),
  ];
  if (isActive) {
    for (let i = 0; i < pCount; i++) {
      children.push(h('circle', {
        key: 'p' + i, r: 4, cx: 0, cy: 0, fill: flowColor,
        ref: (function (idx) { return function (el) { particleRefs.current[idx] = el; }; })(i),
        style: {
          offsetPath: "path('" + edgePath + "')", offsetDistance: reverse ? '100%' : '0%',
          offsetRotate: '0deg', willChange: 'offset-distance',
          filter: 'drop-shadow(0 0 3px ' + flowColor + ')',
        },
      }));
    }
  }
  return h(React.Fragment, null,
    h('g', null, ...children),
    data?.history ? h(EdgeChartOverlay, { labelX, labelY, history: data.history, color }) : null,
  );
}

// ── CUSTOM BEZIER EDGE ────────────────────────────────────────────────────────
function CustomBezierEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, style, data }) {
  const color      = style?.stroke ?? '#2563eb';
  const pathResult = useMemo(
    () => getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition }),
    [sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition]
  );
  const edgePath = pathResult[0];
  const labelX   = pathResult[1];
  const labelY   = pathResult[2];
  const isH      = Math.abs(targetX - sourceX) >= Math.abs(targetY - sourceY);
  const ox = isH ? 0 : 3;
  const oy = isH ? 3 : 0;

  const flowValue  = data?.flowValue    ?? 0;
  const flowSpeed  = data?.flowSpeed    ?? 1;
  const flowColor  = data?.flowColor    ?? color;
  const reverse    = data?.reverse      ?? false;
  const pCount     = Math.max(1, Math.min(5, data?.particleCount ?? 2));
  const isActive   = flowValue > 0;

  const particleRefs = useRef([]);
  const animsRef     = useRef([]);

  useEffect(function () {
    animsRef.current.forEach(function (a) { try { if (a) a.cancel(); } catch (_) {} });
    animsRef.current = [];
    if (!isActive) return;
    const duration  = Math.max(500, Math.min(5000, 2000 / Math.max(0.4, flowSpeed)));
    const keyframes = reverse
      ? [{ offsetDistance: '100%' }, { offsetDistance: '0%' }]
      : [{ offsetDistance: '0%'   }, { offsetDistance: '100%' }];
    const newAnims = [];
    for (let i = 0; i < pCount; i++) {
      const el = particleRefs.current[i];
      if (!el) continue;
      try {
        el.style.offsetPath = "path('" + edgePath + "')";
        newAnims.push(el.animate(keyframes, {
          duration, iterations: Infinity, easing: 'linear',
          delay: -(duration / pCount) * i,
        }));
      } catch (err) { console.warn('[CustomBezierEdge] WAAPI:', err); }
    }
    animsRef.current = newAnims;
    return function () { newAnims.forEach(function (a) { try { if (a) a.cancel(); } catch (_) {} }); };
  }, [edgePath, isActive, flowSpeed, reverse, pCount]);

  const lineOpacity = isActive ? 0.55 : 0.25;
  const lineStyle   = { fill: 'none', stroke: color, strokeWidth: 2, strokeOpacity: lineOpacity, strokeLinecap: 'round' };
  const children    = [
    h('path', { key: 'l1', d: edgePath, style: lineStyle, transform: 'translate(' + ox    + ',' + oy    + ')' }),
    h('path', { key: 'l2', d: edgePath, style: lineStyle, transform: 'translate(' + (-ox) + ',' + (-oy) + ')' }),
  ];
  if (isActive) {
    for (let i = 0; i < pCount; i++) {
      children.push(h('circle', {
        key: 'p' + i, r: 4, cx: 0, cy: 0, fill: flowColor,
        ref: (function (idx) { return function (el) { particleRefs.current[idx] = el; }; })(i),
        style: {
          offsetPath: "path('" + edgePath + "')", offsetDistance: reverse ? '100%' : '0%',
          offsetRotate: '0deg', willChange: 'offset-distance',
          filter: 'drop-shadow(0 0 3px ' + flowColor + ')',
        },
      }));
    }
  }
  return h(React.Fragment, null,
    h('g', null, ...children),
    data?.history ? h(EdgeChartOverlay, { labelX, labelY, history: data.history, color }) : null,
  );
}

// ── TRIPLE ATS EDGE ───────────────────────────────────────────────────────────
function TripleAtsEdge({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, style, data }) {
  const color  = style?.stroke ?? '#10b981';
  const GAP    = 5;
  const useBez = data?.useBezier ?? false;

  const pathResult = useMemo(function () {
    return useBez
      ? getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition })
      : getSmoothStepPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition, borderRadius: 5 });
  }, [sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition, useBez]);
  const edgePath = pathResult[0];
  const labelX   = pathResult[1];
  const labelY   = pathResult[2];

  const isH = Math.abs(targetX - sourceX) >= Math.abs(targetY - sourceY);
  const ox  = isH ? 0 : GAP;
  const oy  = isH ? GAP : 0;

  const flowValue = data?.flowValue ?? 0;
  const flowSpeed = data?.flowSpeed ?? 1;
  const flowColor = data?.flowColor ?? color;
  const reverse   = data?.reverse   ?? false;
  const isActive  = flowValue > 0;

  const particleRef = useRef(null);
  const animRef     = useRef(null);

  useEffect(function () {
    try { if (animRef.current) animRef.current.cancel(); } catch (_) {}
    animRef.current = null;
    if (!isActive) return;
    const duration  = Math.max(500, Math.min(5000, 2000 / Math.max(0.4, flowSpeed)));
    const keyframes = reverse
      ? [{ offsetDistance: '100%' }, { offsetDistance: '0%' }]
      : [{ offsetDistance: '0%'   }, { offsetDistance: '100%' }];
    try {
      if (particleRef.current) {
        particleRef.current.style.offsetPath = "path('" + edgePath + "')";
        animRef.current = particleRef.current.animate(keyframes, { duration, iterations: Infinity, easing: 'linear' });
      }
    } catch (err) { console.warn('[TripleAtsEdge] WAAPI:', err); }
    return function () { try { if (animRef.current) animRef.current.cancel(); } catch (_) {} };
  }, [edgePath, isActive, flowSpeed, reverse]);

  const lineOpacity = isActive ? 0.7 : 0.45;
  const dashStyle   = { fill: 'none', stroke: color, strokeWidth: 2, strokeOpacity: lineOpacity, strokeDasharray: '6 6', strokeLinecap: 'round' };
  const children = [
    h('path', { key: 'l1', d: edgePath, style: dashStyle, transform: 'translate(' + ox    + ',' + oy    + ')' }),
    h('path', { key: 'l2', d: edgePath, style: dashStyle }),
    h('path', { key: 'l3', d: edgePath, style: dashStyle, transform: 'translate(' + (-ox) + ',' + (-oy) + ')' }),
  ];
  if (isActive) {
    children.push(h('circle', {
      key: 'dot', r: 4, cx: 0, cy: 0, fill: flowColor,
      ref: function (el) { particleRef.current = el; },
      style: {
        offsetPath: "path('" + edgePath + "')", offsetDistance: reverse ? '100%' : '0%',
        offsetRotate: '0deg', willChange: 'offset-distance',
        filter: 'drop-shadow(0 0 3px ' + flowColor + ')',
      },
    }));
  }
  return h(React.Fragment, null,
    h('g', null, ...children),
    data?.history ? h(EdgeChartOverlay, { labelX, labelY, history: data.history, color }) : null,
  );
}
