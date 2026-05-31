# 📊 Grafana + redb (metrics-store) pour Monitoring PV Solaire

Guide complet d'installation de Grafana sur Raspberry Pi 5 avec dashboard de comparaison de production photovoltaïque sur 5 ans.

---

## 🚀 Installation Automatique (recommandée)

Un script d'installation idempotent gère **l'intégralité** de la mise en place :
dépôt APT, paquet Grafana, provisioning de la datasource redb (daly-metrics) et du
dashboard PV, ouverture UFW, healthcheck.

```bash
# Depuis ~/Daly-BMS-Rust sur le Pi5
bash scripts/setup-grafana.sh

# Options utiles
bash scripts/setup-grafana.sh --admin-pwd='ChangeMe!2026'   # mot de passe initial
bash scripts/setup-grafana.sh --port=8081                   # port custom
bash scripts/setup-grafana.sh --renderer                    # + image renderer (PDF)
bash scripts/setup-grafana.sh --api-url=http://10.0.0.5:8080 # daly-bms-server distant
sudo bash scripts/setup-grafana.sh --uninstall              # désinstallation
```

Le script déploie :

| Élément | Chemin |
|---------|--------|
| Datasource (auto) | `/etc/grafana/provisioning/datasources/daly-metrics.yaml` |
| Provider dashboards | `/etc/grafana/provisioning/dashboards/daly-bms.yaml` |
| Dashboard PV | `/var/lib/grafana/dashboards/pv-solar-5y.json` |
| Sources | `contrib/grafana/` (versionné dans le repo) |

**Aucun import manuel à faire** — au premier démarrage Grafana, la datasource
`Daly Metrics (redb)` et le dashboard *PV Solaire - Monitoring & Comparaison 5 Ans*
(dossier *PV Solaire*) sont créés automatiquement.

---

## 🖥️ Installation Manuelle (étape par étape)

### 1. Prérequis

```bash
# Mettre à jour le système
sudo apt update && sudo apt upgrade -y

# Installer les dépendances
sudo apt install -y apt-transport-https software-properties-common wget
```

### 2. Ajouter le dépôt Grafana

```bash
# Télécharger la clé GPG
wget -q -O /usr/share/keyrings/grafana.key https://packages.grafana.com/gpg.key

# Ajouter le dépôt (version OSS pour ARM64)
echo "deb [signed-by=/usr/share/keyrings/grafana.key] https://packages.grafana.com/oss/deb stable main" | sudo tee /etc/apt/sources.list.d/grafana.list

# Mettre à jour les paquets
sudo apt update
```

### 3. Installer Grafana

```bash
sudo apt install -y grafana
```

### 4. Démarrer et activer Grafana

```bash
# Démarrer le service
sudo systemctl daemon-reload
sudo systemctl enable grafana-server
sudo systemctl start grafana-server

# Vérifier le statut
sudo systemctl status grafana-server
```

### 5. Accéder à Grafana

- **URL** : `http://<ip-du-pi5>:3000`
- **Login par défaut** : `admin` / `admin`
- **Changer le mot de passe** lors de la première connexion

### 6. (Optionnel) Changer le port

```bash
sudo nano /etc/grafana/grafana.ini
# Modifier : http_port = 3000  →  http_port = 8080
sudo systemctl restart grafana-server
```

---

## 🔌 Connexion à la datasource redb (daly-metrics)

> La datasource est provisionnée automatiquement par `scripts/setup-grafana.sh`.
> La procédure manuelle ci-dessous n'est utile qu'en cas de création à la main.

1. Dans Grafana : **Configuration → Data Sources → Add data source**
2. Sélectionner **Prometheus**
3. **URL** : `http://192.168.1.141:8080` *(adaptez selon votre config)*
4. **Save & Test** → doit afficher "Data source is working"

---

## 📈 Dashboard PV - Comparaison Multi-Années

> ⚠️ **Compatibilité shim PromQL redb** — ce dashboard d'origine a été conçu
> pour MetricsQL (VictoriaMetrics) et ses panels de comparaison annuelle
> utilisent le modificateur **`offset`** (`… offset 1y`) et des **subqueries**,
> que le shim PromQL de redb **ne supporte pas** (cf. `docs/architecture-redb.md`
> §5). Pour les reproduire avec redb : décaler la **fenêtre temporelle côté
> Grafana** (ou via les bornes `start`/`end` de l'API) au lieu d'`offset`, et
> calculer les cumuls **côté client** à partir d'un `query_range`. Importez-le
> tel quel pour la structure, puis adaptez les requêtes des panels comparatifs.

### Import du dashboard

1. **Dashboards → Import → Upload JSON file**
2. Sélectionner le fichier ci-dessous
3. Choisir la data source Daly Metrics (redb)
4. **Import**

### JSON du Dashboard

```json
{
  "annotations": {
    "list": [
      {
        "builtIn": 1,
        "datasource": {
          "type": "grafana",
          "uid": "-- Grafana --"
        },
        "enable": true,
        "hide": true,
        "iconColor": "rgba(0, 211, 255, 1)",
        "name": "Annotations & Alerts",
        "type": "dashboard"
      }
    ]
  },
  "editable": true,
  "fiscalYearStartMonth": 0,
  "graphTooltip": 0,
  "id": null,
  "links": [],
  "liveNow": false,
  "panels": [
    {
      "collapsed": false,
      "gridPos": {
        "h": 1,
        "w": 24,
        "x": 0,
        "y": 0
      },
      "id": 20,
      "panels": [],
      "title": "⚡ Puissance Instantanée",
      "type": "row"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "thresholds"
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "red",
                "value": null
              },
              {
                "color": "yellow",
                "value": 500
              },
              {
                "color": "green",
                "value": 1500
              }
            ]
          },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 4,
        "w": 8,
        "x": 0,
        "y": 1
      },
      "id": 2,
      "options": {
        "colorMode": "value",
        "graphMode": "area",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": {
          "calcs": [
            "lastNotNull"
          ],
          "fields": "",
          "values": false
        },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "total_solar_power",
          "refId": "A"
        }
      ],
      "title": "Puissance PV Totale",
      "type": "stat"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "palette-classic"
          },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "line",
            "fillOpacity": 10,
            "gradientMode": "none",
            "hideFrom": {
              "legend": false,
              "tooltip": false,
              "viz": false
            },
            "insertNulls": false,
            "lineInterpolation": "linear",
            "lineWidth": 1,
            "pointSize": 5,
            "scaleDistribution": {
              "type": "linear"
            },
            "showPoints": "never",
            "spanNulls": false,
            "stacking": {
              "group": "A",
              "mode": "none"
            },
            "thresholdsStyle": {
              "mode": "off"
            }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "green",
                "value": null
              }
            ]
          },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 8,
        "w": 16,
        "x": 8,
        "y": 1
      },
      "id": 3,
      "options": {
        "legend": {
          "calcs": [],
          "displayMode": "list",
          "placement": "bottom",
          "showLegend": true
        },
        "tooltip": {
          "mode": "multi",
          "sort": "none"
        }
      },
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "total_solar_power",
          "legendFormat": "Total",
          "refId": "A"
        }
      ],
      "title": "Courbe Puissance (Temps Réel)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": {
        "h": 1,
        "w": 24,
        "x": 0,
        "y": 9
      },
      "id": 21,
      "panels": [],
      "title": "📅 Comparaison Quotidienne",
      "type": "row"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "thresholds"
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "blue",
                "value": null
              }
            ]
          },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 4,
        "w": 6,
        "x": 0,
        "y": 10
      },
      "id": 4,
      "options": {
        "colorMode": "value",
        "graphMode": "area",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": {
          "calcs": [
            "lastNotNull"
          ],
          "fields": "",
          "values": false
        },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1d])",
          "refId": "A"
        }
      ],
      "title": "Aujourd'hui (Wh)",
      "type": "stat"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "thresholds"
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "purple",
                "value": null
              }
            ]
          },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 4,
        "w": 6,
        "x": 6,
        "y": 10
      },
      "id": 5,
      "options": {
        "colorMode": "value",
        "graphMode": "area",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": {
          "calcs": [
            "lastNotNull"
          ],
          "fields": "",
          "values": false
        },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1d] offset 1d)",
          "refId": "A"
        }
      ],
      "title": "Hier (Wh)",
      "type": "stat"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "thresholds"
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "red",
                "value": null
              },
              {
                "color": "green",
                "value": 0
              }
            ]
          },
          "unit": "percent"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 4,
        "w": 6,
        "x": 12,
        "y": 10
      },
      "id": 6,
      "options": {
        "colorMode": "value",
        "graphMode": "none",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": {
          "calcs": [
            "lastNotNull"
          ],
          "fields": "",
          "values": false
        },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "((increase(total_solar_power[1d]) - increase(total_solar_power[1d] offset 1d)) / increase(total_solar_power[1d] offset 1d)) * 100",
          "refId": "A"
        }
      ],
      "title": "Variation J/J-1 (%)",
      "type": "stat"
    },
    {
      "collapsed": false,
      "gridPos": {
        "h": 1,
        "w": 24,
        "x": 0,
        "y": 14
      },
      "id": 22,
      "panels": [],
      "title": "📆 Comparaison Mensuelle",
      "type": "row"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "palette-classic"
          },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "bars",
            "fillOpacity": 80,
            "gradientMode": "none",
            "hideFrom": {
              "legend": false,
              "tooltip": false,
              "viz": false
            },
            "insertNulls": false,
            "lineInterpolation": "linear",
            "lineWidth": 1,
            "pointSize": 5,
            "scaleDistribution": {
              "type": "linear"
            },
            "showPoints": "never",
            "spanNulls": false,
            "stacking": {
              "group": "A",
              "mode": "none"
            },
            "thresholdsStyle": {
              "mode": "off"
            }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "green",
                "value": null
              }
            ]
          },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 8,
        "w": 24,
        "x": 0,
        "y": 15
      },
      "id": 7,
      "options": {
        "legend": {
          "calcs": [
            "mean",
            "max"
          ],
          "displayMode": "table",
          "placement": "right",
          "showLegend": true
        },
        "tooltip": {
          "mode": "multi",
          "sort": "none"
        }
      },
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[30d])",
          "legendFormat": "Mois en cours",
          "refId": "A"
        },
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[30d] offset 30d)",
          "legendFormat": "Mois précédent",
          "refId": "B"
        }
      ],
      "title": "Production Mensuelle (Wh)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": {
        "h": 1,
        "w": 24,
        "x": 0,
        "y": 23
      },
      "id": 23,
      "panels": [],
      "title": "📊 Comparaison Annuelle - 5 Ans",
      "type": "row"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "palette-classic"
          },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "line",
            "fillOpacity": 0,
            "gradientMode": "none",
            "hideFrom": {
              "legend": false,
              "tooltip": false,
              "viz": false
            },
            "insertNulls": false,
            "lineInterpolation": "smooth",
            "lineWidth": 2,
            "pointSize": 3,
            "scaleDistribution": {
              "type": "linear"
            },
            "showPoints": "auto",
            "spanNulls": false,
            "stacking": {
              "group": "A",
              "mode": "none"
            },
            "thresholdsStyle": {
              "mode": "off"
            }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "green",
                "value": null
              }
            ]
          },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 10,
        "w": 24,
        "x": 0,
        "y": 24
      },
      "id": 8,
      "options": {
        "legend": {
          "calcs": [
            "mean",
            "max",
            "min"
          ],
          "displayMode": "table",
          "placement": "right",
          "showLegend": true
        },
        "tooltip": {
          "mode": "multi",
          "sort": "none"
        }
      },
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1y])",
          "legendFormat": "Année N",
          "refId": "A"
        },
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1y] offset 1y)",
          "legendFormat": "Année N-1",
          "refId": "B"
        },
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1y] offset 2y)",
          "legendFormat": "Année N-2",
          "refId": "C"
        },
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1y] offset 3y)",
          "legendFormat": "Année N-3",
          "refId": "D"
        },
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "increase(total_solar_power[1y] offset 4y)",
          "legendFormat": "Année N-4",
          "refId": "E"
        }
      ],
      "title": "Production Annuelle sur 5 Ans (Comparaison)",
      "type": "timeseries"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "thresholds"
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "red",
                "value": null
              },
              {
                "color": "yellow",
                "value": -5
              },
              {
                "color": "green",
                "value": 0
              }
            ]
          },
          "unit": "percent"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 4,
        "w": 12,
        "x": 0,
        "y": 34
      },
      "id": 9,
      "options": {
        "colorMode": "value",
        "graphMode": "area",
        "justifyMode": "auto",
        "orientation": "auto",
        "reduceOptions": {
          "calcs": [
            "lastNotNull"
          ],
          "fields": "",
          "values": false
        },
        "textMode": "auto"
      },
      "pluginVersion": "10.0.0",
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "((increase(total_solar_power[1y]) - increase(total_solar_power[1y] offset 1y)) / increase(total_solar_power[1y] offset 1y)) * 100",
          "refId": "A"
        }
      ],
      "title": "Dégradation Annuelle N vs N-1 (%)",
      "type": "stat"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "palette-classic"
          },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "bars",
            "fillOpacity": 80,
            "gradientMode": "none",
            "hideFrom": {
              "legend": false,
              "tooltip": false,
              "viz": false
            },
            "insertNulls": false,
            "lineInterpolation": "linear",
            "lineWidth": 1,
            "pointSize": 5,
            "scaleDistribution": {
              "type": "linear"
            },
            "showPoints": "never",
            "spanNulls": false,
            "stacking": {
              "group": "A",
              "mode": "none"
            },
            "thresholdsStyle": {
              "mode": "off"
            }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "green",
                "value": null
              }
            ]
          },
          "unit": "watth"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 8,
        "w": 24,
        "x": 0,
        "y": 38
      },
      "id": 10,
      "options": {
        "legend": {
          "calcs": [
            "sum"
          ],
          "displayMode": "table",
          "placement": "right",
          "showLegend": true
        },
        "tooltip": {
          "mode": "multi",
          "sort": "none"
        }
      },
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "sum(increase(total_solar_power[30d]))",
          "legendFormat": "Total Mensuel",
          "refId": "A"
        }
      ],
      "title": "Production Mensuelle Cumulée (Wh)",
      "type": "timeseries"
    },
    {
      "collapsed": false,
      "gridPos": {
        "h": 1,
        "w": 24,
        "x": 0,
        "y": 46
      },
      "id": 24,
      "panels": [],
      "title": "🌅 Profil Journalier (Comparaison Saisonnière)",
      "type": "row"
    },
    {
      "datasource": {
        "type": "prometheus",
        "uid": "${datasource}"
      },
      "fieldConfig": {
        "defaults": {
          "color": {
            "mode": "palette-classic"
          },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "line",
            "fillOpacity": 10,
            "gradientMode": "none",
            "hideFrom": {
              "legend": false,
              "tooltip": false,
              "viz": false
            },
            "insertNulls": false,
            "lineInterpolation": "smooth",
            "lineWidth": 2,
            "pointSize": 3,
            "scaleDistribution": {
              "type": "linear"
            },
            "showPoints": "auto",
            "spanNulls": false,
            "stacking": {
              "group": "A",
              "mode": "none"
            },
            "thresholdsStyle": {
              "mode": "off"
            }
          },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              {
                "color": "green",
                "value": null
              }
            ]
          },
          "unit": "watt"
        },
        "overrides": []
      },
      "gridPos": {
        "h": 8,
        "w": 24,
        "x": 0,
        "y": 47
      },
      "id": 11,
      "options": {
        "legend": {
          "calcs": [
            "mean"
          ],
          "displayMode": "table",
          "placement": "right",
          "showLegend": true
        },
        "tooltip": {
          "mode": "multi",
          "sort": "none"
        }
      },
      "targets": [
        {
          "datasource": {
            "type": "prometheus",
            "uid": "${datasource}"
          },
          "expr": "avg_over_time(total_solar_power[1d])",
          "legendFormat": "Moyenne Journalière",
          "refId": "A"
        }
      ],
      "title": "Profil de Production Moyen (W)",
      "type": "timeseries"
    }
  ],
  "refresh": "30s",
  "schemaVersion": 38,
  "style": "dark",
  "tags": [
    "pv",
    "solaire",
    "victron",
    "redb"
  ],
  "templating": {
    "list": [
      {
        "current": {
          "selected": false,
          "text": "Prometheus",
          "value": "prometheus"
        },
        "hide": 0,
        "includeAll": false,
        "label": "Data Source",
        "multi": false,
        "name": "datasource",
        "options": [],
        "query": "prometheus",
        "refresh": 1,
        "regex": "",
        "skipUrlSync": false,
        "type": "datasource"
      }
    ]
  },
  "time": {
    "from": "now-30d",
    "to": "now"
  },
  "timepicker": {},
  "timezone": "browser",
  "title": "☀️ PV Solaire - Monitoring & Comparaison 5 Ans",
  "uid": "pv-solar-5y",
  "version": 1,
  "weekStart": ""
}
```

---

## 📄 Génération de Rapports PDF

### Méthode 1 : Export natif Grafana (v9+)

1. Ouvrir le dashboard
2. **Share** (icône en haut) → **Export** → **PDF**
3. Choisir la période (ex: "Last 30 days", "Last 1 year")
4. Télécharger le PDF

### Méthode 2 : Plugin Image Renderer (recommandé)

```bash
# Installation
sudo grafana-cli plugins install grafana-image-renderer
sudo systemctl restart grafana-server

# Utilisation
# Dashboard → Share → Direct link rendered image → Format PDF
```

### Méthode 3 : API Grafana + Script Automatique

```bash
#!/bin/bash
# grafana_pdf_export.sh

GRAFANA_URL="http://localhost:3000"
API_KEY="votre-api-key"
DASHBOARD_UID="pv-solar-5y"
OUTPUT_DIR="/home/pi/reports"
DATE=$(date +%Y-%m-%d)

# Générer PDF via API
curl -H "Authorization: Bearer ${API_KEY}"   "${GRAFANA_URL}/render/d/${DASHBOARD_UID}?width=1920&height=4000&from=now-30d&to=now"   -o "${OUTPUT_DIR}/rapport_pv_${DATE}.pdf"

# Cron pour exécution mensuelle
# 0 1 1 * * /home/pi/grafana_pdf_export.sh
```

---

## 🔧 Requêtes PromQL Utiles

> ⚠️ **Métriques réelles écrites par le projet** (voir `crates/daly-bms-server/src/redb_writes.rs`):
> - `solar_total_w` — gauge, puissance PV totale instantanée (W)
> - `dc_pv_power_w` — gauge, somme MPPT côté DC (W)
> - `pvinv_power_w` — gauge, micro-onduleurs ET112 côté AC (W)
> - `solar_yield_kwh` — compteur journalier remis à 0 chaque jour (kWh)
>
> `increase()` ne s'applique **PAS** aux gauges. Pour l'énergie, utilisez le
> compteur `solar_yield_kwh` (cumul journalier) ou l'approximation
> `avg_over_time(<puissance>[durée]) * heures` (Wh).

> ⚠️ **Limites du shim PromQL redb** — contrairement à MetricsQL/VictoriaMetrics,
> le shim **ne supporte pas** :
> - la fonction **`integrate()`** (MetricsQL) → utiliser `avg_over_time(p[w]) * heures` ;
> - le modificateur **`offset`** → décaler la **fenêtre temporelle côté client**
>   (plage Grafana, ou bornes `start`/`end` de l'API) au lieu d'`offset` ;
> - les **subqueries `[range:step]`** → faire un `query_range` sur la période puis
>   agréger (somme / cumul) **côté client**.
>
> Les lignes marquées ⚠️ ci-dessous reposent sur ces constructions non supportées
> et nécessitent l'un de ces contournements. Réf. `docs/redb-queries.md` et
> `docs/architecture-redb.md` §5.

| Objectif | Requête |
|----------|---------|
| **Puissance totale instantanée** | `solar_total_w` |
| **Production aujourd'hui** | `max_over_time(solar_yield_kwh[24h])` |
| **Production hier** ⚠️ | `max_over_time(solar_yield_kwh[24h])` sur une plage décalée de 24 h **côté client** (pas d'`offset`) |
| **Variation J / J-1 (%)** ⚠️ | calcul **côté client** à partir des valeurs « aujourd'hui » et « hier » (le shim n'a pas d'`offset`) |
| **Production cumulée 30 j** ⚠️ | `query_range` de `max_over_time(solar_yield_kwh[1d])` sur 30 j, puis somme **côté client** (pas de subquery) |
| **Production cumulée 1 an** ⚠️ | idem sur 365 j, somme **côté client** |
| **Comparaison année N vs N-1** ⚠️ | `max_over_time(solar_yield_kwh[1d])` sur deux plages annuelles distinctes **côté client** (pas d'`offset 365d`) |
| **Puissance moyenne 1 h** | `avg_over_time(solar_total_w[1h])` |
| **Pic de puissance du jour** | `max_over_time(solar_total_w[24h])` |
| **Énergie via intégration** | `avg_over_time(solar_total_w[1d]) * 24` (Wh) |

---

## 📋 Récapitulatif Architecture

```
┌─────────────────┐     MQTT      ┌─────────────────┐
│  Victron GX     │──────────────→│   Pi5 compute   │
│  (MQTT Broker)  │               │   (AC, DC...).  │
└─────────────────┘               └────────┬────────┘
                                           │ 
                                           ↓
┌─────────────────┐               ┌──────────────────────────┐
│   Grafana       │←──────────────│ daly-bms-server (redb :8080) │
│   (Visualisation│    PromQL     │   (Stockage metrics-store)   │
│    + PDF)       │               │   Tiering 30j/365j/5 ans     │
└─────────────────┘               └──────────────────────────┘
```

---

## 📝 Notes

- **Rétention redb (tiering automatique)** : raw 30 jours / hourly 365 jours / daily 5 ans — géré nativement par le crate `metrics-store`, aucun flag à configurer
- **Espace disque** : le tiering downsample automatiquement les données anciennes (base `/mnt/nvme/daly-bms/metrics.redb`) — surveiller avec `du -sh /mnt/nvme/daly-bms/metrics.redb`
- **Backup** : sauvegarder le fichier `metrics.redb` (sur NVMe) pour un archivage externe

---

*Dashboard créé pour monitoring PV Victron + redb sur Raspberry Pi 5*
