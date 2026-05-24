#!/usr/bin/env python3
"""Generate 16-bms-detail.json Grafana dashboard (provisioning format)."""
import json, textwrap

DS = {"type": "prometheus", "uid": "daly-metrics"}

def stat_panel(pid, title, expr, gridPos, unit="none", thresholds=None, mappings=None, color_mode="value", text_mode="auto", decimals=None, graph_mode="none"):
    p = {
        "id": pid, "type": "stat", "title": title,
        "gridPos": gridPos,
        "datasource": DS,
        "targets": [{"datasource": DS, "expr": expr, "legendFormat": "", "refId": "A", "instant": True, "range": False}],
        "options": {
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "orientation": "auto", "textMode": text_mode,
            "colorMode": color_mode, "graphMode": graph_mode,
            "justifyMode": "auto"
        },
        "fieldConfig": {
            "defaults": {
                "unit": unit,
                "thresholds": thresholds or {"mode": "absolute", "steps": [{"value": None, "color": "green"}]},
                "mappings": mappings or []
            },
            "overrides": []
        }
    }
    if decimals is not None:
        p["fieldConfig"]["defaults"]["decimals"] = decimals
    return p

def gauge_panel(pid, title, expr, gridPos, unit="percent", min_val=0, max_val=100, thresholds=None):
    return {
        "id": pid, "type": "gauge", "title": title,
        "gridPos": gridPos,
        "datasource": DS,
        "targets": [{"datasource": DS, "expr": expr, "legendFormat": "", "refId": "A", "instant": True, "range": False}],
        "options": {
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "showThresholdLabels": False, "showThresholdMarkers": True,
            "orientation": "auto"
        },
        "fieldConfig": {
            "defaults": {
                "unit": unit, "min": min_val, "max": max_val,
                "thresholds": thresholds or {"mode": "absolute", "steps": [{"value": None, "color": "green"}]},
                "mappings": []
            },
            "overrides": []
        }
    }

def row_panel(pid, title, y, collapsed=False):
    return {"id": pid, "type": "row", "title": title, "gridPos": {"h": 1, "w": 24, "x": 0, "y": y}, "collapsed": collapsed, "panels": []}

def ts_panel(pid, title, targets_list, gridPos, unit="short", fill=10, draw="line"):
    targets = []
    for i, (expr, legend) in enumerate(targets_list):
        targets.append({"datasource": DS, "expr": expr, "legendFormat": legend, "refId": chr(65+i)})
    return {
        "id": pid, "type": "timeseries", "title": title,
        "gridPos": gridPos,
        "datasource": DS,
        "targets": targets,
        "options": {
            "tooltip": {"mode": "multi", "sort": "none"},
            "legend": {"displayMode": "list", "placement": "bottom", "showLegend": True}
        },
        "fieldConfig": {
            "defaults": {
                "unit": unit,
                "custom": {"lineWidth": 1, "fillOpacity": fill, "drawStyle": draw, "spanNulls": False}
            },
            "overrides": []
        }
    }

# ── ECharts bar chart JS code ─────────────────────────────────────────────
BAR_CODE = textwrap.dedent(r"""
const cells = [];
for (const frame of data.series) {
  const vf = frame.fields.find(f => f.type === 'number');
  if (!vf || !vf.labels || !vf.labels.cell) continue;
  const cn = parseInt(vf.labels.cell, 10);
  const vals = vf.values;
  const lv = vals.length > 0 ? vals[vals.length - 1] : null;
  if (lv === null || lv === undefined || isNaN(lv)) continue;
  cells.push({ num: cn, name: 'C' + cn, value: lv });
}
cells.sort((a, b) => a.num - b.num);
if (cells.length === 0) return {};

let minV = Infinity, maxV = -Infinity;
cells.forEach(c => { if (c.value < minV) minV = c.value; if (c.value > maxV) maxV = c.value; });
const avg = cells.reduce((s, c) => s + c.value, 0) / cells.length;
const deltaMv = (maxV - minV) * 1000;
const yMin = Math.max(minV - 0.030, 2.5);
const yMax = Math.min(maxV + 0.030, 4.3);
const deltaColor = deltaMv > 100 ? '#f85149' : deltaMv > 50 ? '#e3b341' : '#3fb950';

const categories = cells.map(c => c.name);
const barData = cells.map(c => {
  const isMin = Math.abs(c.value - minV) < 0.0001;
  const isMax = Math.abs(c.value - maxV) < 0.0001;
  const color = isMin ? '#f85149' : isMax ? '#3fb950' : '#58a6ff';
  const label = isMin ? 'MIN' : isMax ? 'MAX' : '';
  return {
    value: c.value,
    itemStyle: { color: color, borderRadius: [3, 3, 0, 0] },
    label: { show: !!label, formatter: label, position: 'top', fontSize: 9, fontWeight: 'bold', color: color }
  };
});

return {
  backgroundColor: 'transparent',
  animation: false,
  title: { text: 'Δ = ' + deltaMv.toFixed(0) + ' mV', right: '1%', top: '2%',
           textStyle: { color: deltaColor, fontSize: 12, fontWeight: 'bold' } },
  tooltip: { trigger: 'axis', formatter: '{b}: {c} V',
             backgroundColor: '#161b22', borderColor: '#30363d',
             textStyle: { color: '#c9d1d9', fontSize: 11 } },
  grid: { left: '1%', right: '5%', top: '14%', bottom: '8%', containLabel: true },
  xAxis: { type: 'category', data: categories,
            axisLabel: { color: '#8b949e', fontSize: 10 },
            axisLine: { lineStyle: { color: '#30363d' } } },
  yAxis: { type: 'value',
            min: parseFloat(yMin.toFixed(3)), max: parseFloat(yMax.toFixed(3)),
            splitNumber: 4,
            axisLabel: { color: '#8b949e', formatter: '{value} V', fontSize: 10 },
            splitLine: { lineStyle: { color: '#21262d', type: 'dashed' } } },
  series: [{
    type: 'bar', data: barData, barMaxWidth: 28,
    markLine: {
      silent: true, symbol: 'none',
      data: [{ yAxis: parseFloat(avg.toFixed(4)),
               lineStyle: { color: '#e3b341', type: 'dashed', width: 1.5 },
               label: { show: true, formatter: 'moy ' + avg.toFixed(3) + ' V',
                        color: '#e3b341', fontSize: 9, position: 'insideEndTop' } }]
    }
  }]
};
""").strip()

# ── ECharts boxplot JS code ───────────────────────────────────────────────
BOXPLOT_CODE = textwrap.dedent(r"""
const cellData = {};
const balanceData = {};

for (const frame of data.series) {
  const vf = frame.fields.find(f => f.type === 'number');
  if (!vf || !vf.labels || !vf.labels.cell) continue;
  const cell = vf.labels.cell;
  if (frame.refId === 'B') {
    const vals = vf.values;
    balanceData[cell] = vals.length > 0 && vals[vals.length - 1] > 0.5;
    continue;
  }
  const vals = Array.from(vf.values).filter(v => v !== null && v !== undefined && !isNaN(v));
  if (vals.length > 0) cellData[cell] = vals;
}

const keys = Object.keys(cellData).map(k => ({ num: parseInt(k, 10), key: k })).sort((a, b) => a.num - b.num);
if (keys.length === 0) return {};

let curMin = { cell: '', val: Infinity };
let curMax = { cell: '', val: -Infinity };
keys.forEach(c => {
  const v = cellData[c.key];
  const last = v[v.length - 1];
  if (last < curMin.val) curMin = { cell: c.key, val: last };
  if (last > curMax.val) curMax = { cell: c.key, val: last };
});

function pct(sorted, p) {
  const i = (p / 100) * (sorted.length - 1);
  const lo = Math.floor(i), hi = Math.ceil(i);
  return lo === hi ? sorted[lo] : sorted[lo] + (sorted[hi] - sorted[lo]) * (i - lo);
}

const categories = [];
const boxD = [];
const scatterD = [];

keys.forEach(c => {
  const vals = cellData[c.key].slice().sort((a, b) => a - b);
  const n = vals.length;
  const mn = vals[0], mx = vals[n - 1];
  const q1 = pct(vals, 25), md = pct(vals, 50), q3 = pct(vals, 75);
  const isMin = c.key === curMin.cell;
  const isMax = c.key === curMax.cell;
  const fill = isMin ? 'rgba(248,81,73,0.25)' : isMax ? 'rgba(63,185,80,0.25)' : 'rgba(88,166,255,0.15)';
  const border = isMin ? '#f85149' : isMax ? '#3fb950' : '#58a6ff';
  categories.push('C' + c.num);
  boxD.push({ value: [mn, q1, md, q3, mx], itemStyle: { color: fill, borderColor: border, borderWidth: 2 } });
  if (balanceData[c.key]) {
    scatterD.push({ value: ['C' + c.num, mx + (mx - mn) * 0.08 + 0.001] });
  }
});

const sampleCount = cellData[keys[0].key] ? cellData[keys[0].key].length : 0;
return {
  backgroundColor: 'transparent',
  animation: false,
  title: { text: 'Distribution par cellule (' + sampleCount + ' points)',
           textStyle: { color: '#8b949e', fontSize: 11, fontWeight: 'normal' }, top: '2%' },
  tooltip: { trigger: 'item', backgroundColor: '#161b22', borderColor: '#30363d',
             textStyle: { color: '#c9d1d9', fontSize: 11 } },
  grid: { left: '1%', right: '2%', top: '14%', bottom: '8%', containLabel: true },
  xAxis: { type: 'category', data: categories,
            axisLabel: { color: '#8b949e', fontSize: 10 },
            axisLine: { lineStyle: { color: '#30363d' } } },
  yAxis: { type: 'value', scale: true,
            axisLabel: { color: '#8b949e', formatter: '{value} V', fontSize: 10 },
            splitLine: { lineStyle: { color: '#21262d', type: 'dashed' } } },
  series: [
    { type: 'boxplot', data: boxD, boxWidth: ['20%', '45%'],
      itemStyle: { color: 'rgba(88,166,255,0.15)', borderColor: '#58a6ff', borderWidth: 1.5 },
      emphasis: { itemStyle: { color: 'rgba(88,166,255,0.30)', borderColor: '#58a6ff', borderWidth: 2 } } },
    { type: 'scatter', data: scatterD, symbol: 'diamond', symbolSize: 10,
      itemStyle: { color: '#e3b341' }, z: 10,
      label: { show: true, formatter: '⚡', position: 'top', fontSize: 10, color: '#e3b341' } }
  ]
};
""").strip()

def echarts_panel(pid, title, gridPos, code, targets):
    return {
        "id": pid, "type": "volkovlabs-echarts-panel", "title": title,
        "gridPos": gridPos,
        "datasource": DS,
        "targets": targets,
        "options": {
            "renderer": "svg",
            "getOption": code,
            "themeEditor": {"config": "{}", "height": 0}
        },
        "fieldConfig": {"defaults": {}, "overrides": []}
    }


# ═════════════════════════════════════════════════════════════════════════════
# Build dashboard
# ═════════════════════════════════════════════════════════════════════════════
panels = []
pid = 1

# ── Row 1: KPIs principaux ──────────────────────────────────────────────────
panels.append(row_panel(pid, "KPIs principaux", 0)); pid += 1

W, H = 4, 4
kpis = [
    ("Tension pack", 'bms_voltage{bms_id="$bms_id"}', "volt", {"mode": "absolute", "steps": [{"value": None, "color": "blue"}]}, None, 1),
    ("Courant", 'bms_current{bms_id="$bms_id"}', "amp",
     {"mode": "absolute", "steps": [{"value": None, "color": "blue"}, {"value": 0, "color": "green"}]}, None, 1),
    ("Puissance", 'bms_power{bms_id="$bms_id"}', "watt", {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, None, 0),
]
for i, (title, expr, unit, thr, maps, dec) in enumerate(kpis):
    panels.append(stat_panel(pid, title, expr, {"h": H, "w": W, "x": i*W, "y": 1}, unit=unit, thresholds=thr, decimals=dec)); pid += 1

# SOC gauge
panels.append(gauge_panel(pid, "SOC", 'bms_soc{bms_id="$bms_id"}', {"h": H, "w": W, "x": 12, "y": 1},
    thresholds={"mode": "absolute", "steps": [
        {"value": None, "color": "red"}, {"value": 15, "color": "orange"},
        {"value": 25, "color": "yellow"}, {"value": 40, "color": "green"}
    ]})); pid += 1

# SOH
panels.append(stat_panel(pid, "SOH", 'bms_soh{bms_id="$bms_id"}', {"h": H, "w": W, "x": 16, "y": 1},
    unit="percent",
    thresholds={"mode": "absolute", "steps": [
        {"value": None, "color": "red"}, {"value": 70, "color": "yellow"}, {"value": 85, "color": "green"}
    ]}, decimals=1)); pid += 1

# T° max
panels.append(stat_panel(pid, "T° max", 'bms_temp_max{bms_id="$bms_id"}', {"h": H, "w": W, "x": 20, "y": 1},
    unit="celsius",
    thresholds={"mode": "absolute", "steps": [
        {"value": None, "color": "green"}, {"value": 35, "color": "yellow"}, {"value": 45, "color": "red"}
    ]}, decimals=1)); pid += 1

# Row 2 KPIs
kpis2 = [
    ("Δ cellules (mV)", 'bms_cell_delta_mv{bms_id="$bms_id"}', "none",
     {"mode": "absolute", "steps": [{"value": None, "color": "green"}, {"value": 50, "color": "yellow"}, {"value": 100, "color": "red"}]}, 0),
    ("Capacite SOC", 'bms_capacity_remaining_ah{bms_id="$bms_id"}', "amph",
     {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, 1),
    ("Cap. BMS", 'bms_reported_capacity_ah{bms_id="$bms_id"}', "amph",
     {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, 1),
    ("Autonomie", 'bms_time_to_go_secs{bms_id="$bms_id"} / 3600', "h",
     {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, 1),
    ("Cycles", 'bms_charge_cycles{bms_id="$bms_id"}', "none",
     {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, 0),
    ("Cellules", 'count(bms_cell_voltage{bms_id="$bms_id"})', "none",
     {"mode": "absolute", "steps": [{"value": None, "color": "text"}]}, 0),
]
for i, (title, expr, unit, thr, dec) in enumerate(kpis2):
    panels.append(stat_panel(pid, title, expr, {"h": H, "w": W, "x": i*W, "y": 5}, unit=unit, thresholds=thr, decimals=dec))
    pid += 1

# ── Row 2: MOS & Extremes ───────────────────────────────────────────────────
panels.append(row_panel(pid, "MOS & Extremes cellules", 9)); pid += 1

# MOS Charge
mos_mappings_chg = [
    {"type": "value", "options": {"0": {"text": "CHG ✗", "color": "red"}, "1": {"text": "CHG ✓", "color": "green"}}}
]
panels.append(stat_panel(pid, "MOS Charge", 'bms_charge_mos{bms_id="$bms_id"}',
    {"h": 3, "w": 4, "x": 0, "y": 10}, mappings=mos_mappings_chg, color_mode="background")); pid += 1

mos_mappings_dch = [
    {"type": "value", "options": {"0": {"text": "DCH ✗", "color": "red"}, "1": {"text": "DCH ✓", "color": "green"}}}
]
panels.append(stat_panel(pid, "MOS Decharge", 'bms_discharge_mos{bms_id="$bms_id"}',
    {"h": 3, "w": 4, "x": 4, "y": 10}, mappings=mos_mappings_dch, color_mode="background")); pid += 1

# Min cell
panels.append(stat_panel(pid, "Cell Min", 'bms_min_cell_voltage{bms_id="$bms_id"}',
    {"h": 3, "w": 8, "x": 8, "y": 10}, unit="volt",
    thresholds={"mode": "absolute", "steps": [{"value": None, "color": "red"}]}, decimals=3)); pid += 1

# Max cell
panels.append(stat_panel(pid, "Cell Max", 'bms_max_cell_voltage{bms_id="$bms_id"}',
    {"h": 3, "w": 8, "x": 16, "y": 10}, unit="volt",
    thresholds={"mode": "absolute", "steps": [{"value": None, "color": "yellow"}]}, decimals=3)); pid += 1

# ── Row 3: Bar chart cellules ────────────────────────────────────────────────
panels.append(row_panel(pid, "Tensions cellules (instantane)", 13)); pid += 1

bar_targets = [
    {"datasource": DS, "expr": 'bms_cell_voltage{bms_id="$bms_id"}', "legendFormat": "C{{cell}}", "refId": "A",
     "instant": True, "range": False}
]
panels.append(echarts_panel(pid, "Tensions cellules", {"h": 10, "w": 24, "x": 0, "y": 14}, BAR_CODE, bar_targets)); pid += 1

# ── Row 4: Boxplot ───────────────────────────────────────────────────────────
panels.append(row_panel(pid, "Distribution historique par cellule", 24)); pid += 1

boxplot_targets = [
    {"datasource": DS, "expr": 'bms_cell_voltage{bms_id="$bms_id"}', "legendFormat": "C{{cell}}", "refId": "A",
     "instant": False, "range": True},
    {"datasource": DS, "expr": 'bms_cell_balancing{bms_id="$bms_id"}', "legendFormat": "Bal {{cell}}", "refId": "B",
     "instant": True, "range": False}
]
panels.append(echarts_panel(pid, "Distribution historique", {"h": 10, "w": 24, "x": 0, "y": 25}, BOXPLOT_CODE, boxplot_targets)); pid += 1

# ── Row 5: Historique temporel ───────────────────────────────────────────────
panels.append(row_panel(pid, "Historique temporel", 35)); pid += 1

panels.append(ts_panel(pid, "Tension Pack", [
    ('bms_voltage{bms_id="$bms_id"}', "Tension")
], {"h": 8, "w": 24, "x": 0, "y": 36}, unit="volt")); pid += 1

panels.append(ts_panel(pid, "Courant", [
    ('bms_current{bms_id="$bms_id"}', "Courant")
], {"h": 8, "w": 12, "x": 0, "y": 44}, unit="amp")); pid += 1

panels.append(ts_panel(pid, "Puissance", [
    ('bms_power{bms_id="$bms_id"}', "Puissance")
], {"h": 8, "w": 12, "x": 12, "y": 44}, unit="watt")); pid += 1

panels.append(ts_panel(pid, "Tensions cellules individuelles", [
    ('bms_cell_voltage{bms_id="$bms_id"}', "C{{cell}}")
], {"h": 10, "w": 24, "x": 0, "y": 52}, unit="volt")); pid += 1

panels.append(ts_panel(pid, "Delta cellules (max-min)", [
    ('bms_cell_delta_mv{bms_id="$bms_id"}', "Delta")
], {"h": 8, "w": 12, "x": 0, "y": 62}, unit="none")); pid += 1

panels.append(ts_panel(pid, "Equilibrage actif", [
    ('bms_balancing_active{bms_id="$bms_id"}', "Balancing")
], {"h": 8, "w": 12, "x": 12, "y": 62}, unit="bool")); pid += 1

# ── Row 6: Temperatures ─────────────────────────────────────────────────────
panels.append(row_panel(pid, "Temperatures", 70)); pid += 1

panels.append(ts_panel(pid, "Temperatures BMS", [
    ('bms_temp_max{bms_id="$bms_id"}', "T° max"),
    ('bms_temp_min{bms_id="$bms_id"}', "T° min"),
    ('bms_mos_temp_c{bms_id="$bms_id"}', "T° MOSFET"),
], {"h": 8, "w": 24, "x": 0, "y": 71}, unit="celsius")); pid += 1

# ── Row 7: MOSFET & Alarmes ─────────────────────────────────────────────────
panels.append(row_panel(pid, "MOSFET & Alarmes", 79)); pid += 1

panels.append(ts_panel(pid, "Etats MOSFET", [
    ('bms_charge_mos{bms_id="$bms_id"}', "Charge MOS"),
    ('bms_discharge_mos{bms_id="$bms_id"}', "Discharge MOS"),
], {"h": 6, "w": 12, "x": 0, "y": 80}, unit="bool")); pid += 1

panels.append(ts_panel(pid, "Limites de charge", [
    ('bms_max_charge_current{bms_id="$bms_id"}', "Max Charge (A)"),
    ('bms_max_discharge_current{bms_id="$bms_id"}', "Max Discharge (A)"),
], {"h": 6, "w": 12, "x": 12, "y": 80}, unit="amp")); pid += 1

# Alarm stats
alarm_metrics = [
    "bms_alarm_low_voltage", "bms_alarm_high_voltage", "bms_alarm_low_soc",
    "bms_alarm_cell_imbalance", "bms_alarm_high_temp", "bms_alarm_low_temp",
    "bms_alarm_high_charge_current", "bms_alarm_high_discharge_current",
]
alarm_targets = []
for i, m in enumerate(alarm_metrics):
    label = m.replace("bms_alarm_", "").replace("_", " ").title()
    alarm_targets.append({"datasource": DS, "expr": f'{m}{{bms_id="$bms_id"}}', "legendFormat": label, "refId": chr(65+i)})

panels.append({
    "id": pid, "type": "stat", "title": "Alarmes actives",
    "gridPos": {"h": 6, "w": 24, "x": 0, "y": 86},
    "datasource": DS,
    "targets": alarm_targets,
    "options": {
        "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
        "orientation": "horizontal", "textMode": "auto",
        "colorMode": "background", "graphMode": "none"
    },
    "fieldConfig": {
        "defaults": {
            "thresholds": {"mode": "absolute", "steps": [{"value": None, "color": "green"}, {"value": 1, "color": "red"}]},
            "mappings": [{"type": "value", "options": {"0": {"text": "OK", "color": "green"}, "1": {"text": "ALARME", "color": "red"}}}]
        },
        "overrides": []
    }
}); pid += 1

# ═════════════════════════════════════════════════════════════════════════════
# Dashboard wrapper
# ═════════════════════════════════════════════════════════════════════════════
dashboard = {
    "annotations": {"list": []},
    "description": "Detail batterie BMS Daly — KPIs, tensions cellules (ECharts), distribution, historique",
    "editable": True,
    "fiscalYearStartMonth": 0,
    "graphTooltip": 1,
    "id": None,
    "links": [
        {"title": "BMS Overview", "url": "/d/bms-batteries-daly/bms-batteries-daly", "type": "link", "icon": "external link", "targetBlank": False}
    ],
    "panels": panels,
    "schemaVersion": 39,
    "tags": ["daly-bms", "battery", "detail", "echarts"],
    "templating": {
        "list": [
            {
                "name": "bms_id",
                "type": "custom",
                "label": "Batterie",
                "current": {"text": "BMS-360Ah (0x01)", "value": "0x01", "selected": True},
                "options": [
                    {"text": "BMS-360Ah (0x01)", "value": "0x01", "selected": True},
                    {"text": "BMS-320Ah (0x02)", "value": "0x02", "selected": False}
                ],
                "query": "0x01 : BMS-360Ah (0x01), 0x02 : BMS-320Ah (0x02)",
                "hide": 0,
                "includeAll": False,
                "multi": False
            }
        ]
    },
    "time": {"from": "now-1h", "to": "now"},
    "timepicker": {},
    "timezone": "browser",
    "title": "BMS — Detail Batterie",
    "uid": "bms-detail-battery",
    "version": 1,
    "refresh": "30s"
}

out = "/home/user/Daly-BMS-Rust/contrib/grafana/dashboards/16-bms-detail.json"
with open(out, "w") as f:
    json.dump(dashboard, f, indent=2, ensure_ascii=False)

print(f"Generated {out}")
print(f"  Panels: {len(panels)}")
print(f"  Size: {len(json.dumps(dashboard))} bytes")
