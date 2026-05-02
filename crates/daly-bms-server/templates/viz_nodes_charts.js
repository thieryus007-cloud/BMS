// ── SPIRAL RACE NODE ──────────────────────────────────────────────────────────
// 3 anneaux concentriques plein-cercle animés :
//   anneau extérieur = Production journalière (kWh, max 40)
//   anneau milieu    = SOC batteries (%, max 100)
//   anneau intérieur = Irradiance instantanée (W/m², max 1200)
const SpiralRaceNode = function SpiralRaceNode({ data }) {
  const chartRef = useRef(null);
  const instRef  = useRef(null);

  const prodKwh = data.totalYieldKwh ?? data.prodKwh ?? 0;
  const soc     = data.soc          ?? 0;
  const irrWm2  = data.irrWm2       ?? 0;

  useEffect(function () {
    if (!chartRef.current || !window.echarts) return;
    if (!instRef.current) {
      instRef.current = window.echarts.init(chartRef.current, null, { renderer: 'canvas' });
    }
    const chart = instRef.current;

    const socCol = soc > 60 ? '#22c55e' : soc > 30 ? '#f59e0b' : '#ef4444';

    // Arc 270° (220° → -40°) : espace en bas pour afficher les labels de chaque anneau
    // Traits fins, animation très progressive
    const mkGauge = (radius, max, value, color, label, unit, titleY, detailY) => ({
      type: 'gauge',
      radius,
      startAngle: 220,
      endAngle:   -40,
      min: 0, max,
      clockwise: true,
      animation: true,
      animationDuration: 2500,
      animationDurationUpdate: 1500,
      animationEasing: 'cubicOut',
      animationEasingUpdate: 'cubicInOut',
      pointer: { show: false },
      progress: {
        show: true, width: 6, roundCap: true, clip: false,
        itemStyle: { color }
      },
      axisLine: {
        show: true,
        lineStyle: { width: 6, color: [[1, 'rgba(148,163,184,0.12)']] }
      },
      axisTick: { show: false }, splitLine: { show: false },
      axisLabel: { show: false },
      title: {
        show: true,
        text: label,
        color: '#94a3b8',
        fontSize: 8,
        offsetCenter: ['0%', titleY]
      },
      detail: {
        show: true,
        formatter: function(v) { return (Math.round(v * 10) / 10) + ' ' + unit; },
        color: color,
        fontSize: 10,
        fontWeight: 'bold',
        offsetCenter: ['0%', detailY]
      },
      data: [{ value: Math.max(0, value) }]
    });

    chart.setOption({
      backgroundColor: 'transparent',
      series: [
        mkGauge('88%',  40,   prodKwh, '#fbbf24', 'Production', 'kWh',  '78%', '90%'),
        mkGauge('65%', 100,   soc,     socCol,    'SOC',        '%',    '48%', '60%'),
        mkGauge('42%', 1200,  irrWm2,  '#38bdf8', 'Irradiance', 'W/m²', '18%', '30%'),
        // Tick marks radiaux fins traversant les 3 anneaux sans atteindre le centre
        {
          type: 'gauge', radius: '88%',
          startAngle: 220, endAngle: -40,
          min: 0, max: 100, splitNumber: 8,
          clockwise: true, animation: false,
          pointer: { show: false },
          progress: { show: false },
          axisLine: { show: false },
          axisTick: { show: false },
          splitLine: {
            show: true, length: 55, distance: 0,
            lineStyle: { width: 0.5, color: 'rgba(148,163,184,0.28)' }
          },
          axisLabel: { show: false },
          title: { show: false },
          detail: { show: false },
          data: [{ value: 0 }]
        },
      ]
    }, { notMerge: false, lazyUpdate: false });
  }, [prodKwh, soc, irrWm2]); // eslint-disable-line

  useEffect(function () {
    return function () {
      if (instRef.current) {
        try { instRef.current.dispose(); } catch (_) {}
        instRef.current = null;
      }
    };
  }, []);

  const socCol = soc > 60 ? '#22c55e' : soc > 30 ? '#f59e0b' : '#ef4444';

  return h('div', { className: 'spiral-node' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('source', Position.Right,  'sr'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Left,   'sl', { top: '68%' }),
    h('div', { className: 'spiral-hdr' },
      h('span', { className: 'spiral-title' }, '☀ Production · SOC · Météo')
    ),
    h('div', { ref: chartRef, className: 'spiral-chart' })
  );
};

// ── SANKEY NODE ───────────────────────────────────────────────────────────────
// Mode production  : Sources = Micro-ond + MPPT  → Batteries(charge) + Maison + Export
// Mode décharge    : Source  = Batteries(disch.) → Maison + Réseau
const SankeyNode = function SankeyNode({ data }) {
  const chartRef   = useRef(null);
  const instRef    = useRef(null);
  const prevKeyRef = useRef('');

  const microW        = Math.max(0, data.microPwr        ?? 0);
  const mpptW         = Math.max(0, data.mpptPwr         ?? 0);
  const batChargeW    = Math.max(0, data.batChargePwr    ?? data.batPwr ?? 0);
  const batDischW     = Math.max(0, data.batDischargePwr ?? 0);
  const loadW         = Math.max(0, data.loadPwr         ?? 0);
  const totalIn       = microW + mpptW;
  const isProduction  = totalIn >= 10;
  const isDischarge   = !isProduction && batDischW >= 10;

  const buildSankey = function(srcNodes, sinkNodes) {
    const structKey = srcNodes.map(n => n.name).join(',') + '|' + sinkNodes.map(n => n.name).join(',');
    const structChanged = structKey !== prevKeyRef.current;
    prevKeyRef.current = structKey;
    const sinkTotal = sinkNodes.reduce((s, n) => s + n.w, 0) || 1;
    const eNodes = [
      ...srcNodes.map(n  => ({ name: n.name, itemStyle: { color: n.color } })),
      ...sinkNodes.map(n => ({ name: n.name, itemStyle: { color: n.color } })),
    ];
    const eLinks = [];
    srcNodes.forEach(function (src) {
      sinkNodes.forEach(function (sink) {
        const v = Math.round(src.w * (sink.w / sinkTotal));
        if (v > 1) eLinks.push({ source: src.name, target: sink.name, value: v });
      });
    });
    return { eNodes, eLinks, structChanged };
  };

  useEffect(function () {
    if (!chartRef.current || !window.echarts) return;
    if (!instRef.current) {
      instRef.current = window.echarts.init(chartRef.current, null, { renderer: 'canvas' });
    }
    const chart = instRef.current;

    if (!isProduction && !isDischarge) {
      chart.setOption({
        backgroundColor: 'transparent',
        series: [],
        graphic: [{ type: 'text', left: 'center', top: 'middle',
          style: { text: 'Pas de production\nni de décharge', fill: '#64748b', fontSize: 10 } }]
      }, { notMerge: true });
      prevKeyRef.current = '';
      return;
    }

    let srcNodes = [], sinkNodes = [];

    if (isProduction) {
      // Mode production : PV → batterie/maison/export
      const batOut  = Math.min(batChargeW, totalIn);
      const loadOut = Math.min(loadW, Math.max(0, totalIn - batOut));
      const expOut  = Math.max(0, totalIn - batOut - loadOut);
      if (microW > 5) srcNodes.push({ name: 'Micro-ond', w: microW, color: '#3b82f6' });
      if (mpptW  > 5) srcNodes.push({ name: 'MPPT',      w: mpptW,  color: '#f59e0b' });
      if (batOut > 5) sinkNodes.push({ name: 'Batteries', w: batOut,  color: '#22c55e' });
      if (loadOut> 5) sinkNodes.push({ name: 'Maison',    w: loadOut, color: '#a855f7' });
      if (expOut > 5) sinkNodes.push({ name: 'Export',    w: expOut,  color: '#0ea5e9' });
      if (sinkNodes.length === 0) sinkNodes.push({ name: 'Consomm.', w: totalIn, color: '#64748b' });
    } else {
      // Mode décharge : Batteries → maison/réseau
      srcNodes.push({ name: 'Batteries', w: batDischW, color: '#f97316' });
      if (loadW > 5)          sinkNodes.push({ name: 'Maison',  w: Math.min(loadW, batDischW), color: '#a855f7' });
      const reste = batDischW - (sinkNodes[0]?.w ?? 0);
      if (reste > 5)          sinkNodes.push({ name: 'Réseau',  w: reste, color: '#0ea5e9' });
      if (sinkNodes.length === 0) sinkNodes.push({ name: 'Consomm.', w: batDischW, color: '#64748b' });
    }

    const { eNodes, eLinks, structChanged } = buildSankey(srcNodes, sinkNodes);

    chart.setOption({
      backgroundColor: 'transparent',
      animation: true,
      animationDuration: 900,
      animationDurationUpdate: 500,
      animationEasing: 'cubicOut',
      graphic: [],
      series: [{
        type: 'sankey',
        left: '5%', right: '15%', top: '10%', bottom: '10%',
        orient: 'horizontal',
        nodeAlign: 'center',
        nodeWidth: 12,
        nodeGap: 16,
        emphasis: { focus: 'adjacency' },
        label: {
          show: true, fontSize: 9, color: '#94a3b8', position: 'right',
          formatter: function (p) { return p.name + '\n' + Math.round(p.value) + ' W'; }
        },
        lineStyle: { color: 'gradient', opacity: 0.45, curveness: 0.5 },
        data: eNodes,
        links: eLinks
      }]
    }, { notMerge: structChanged, lazyUpdate: false });
  }, [microW, mpptW, batChargeW, batDischW, loadW, isProduction, isDischarge]);

  useEffect(function () {
    return function () {
      if (instRef.current) {
        try { instRef.current.dispose(); } catch (_) {}
        instRef.current = null;
      }
    };
  }, []);

  const displayW = isProduction ? Math.round(totalIn) : isDischarge ? Math.round(batDischW) : 0;
  const modeLabel = isDischarge && !isProduction ? '🔋 Décharge' : 'Flux Énergétique';

  return h('div', { className: 'sankey-node' },
    mkHandle('target', Position.Top,    'tt'),
    mkHandle('source', Position.Bottom, 'sb'),
    mkHandle('target', Position.Left,   'tl'),
    mkHandle('source', Position.Right,  'sr'),
    h('div', { className: 'sankey-hdr' },
      h('span', { className: 'sankey-icon' }, isDischarge && !isProduction ? '🔋' : '⚡'),
      h('span', { className: 'sankey-title' }, modeLabel),
      h('span', { className: 'sankey-total' }, displayW > 0 ? displayW + ' W' : '—')
    ),
    h('div', { ref: chartRef, className: 'sankey-chart' })
  );
};
