## Installation

```bash
# Grafana (si Debian/Ubuntu)
sudo apt-get install -y apt-transport-https software-properties-common wget
wget -q -O /usr/share/keyrings/grafana.key https://packages.grafana.com/gpg.key
echo "deb [signed-by=/usr/share/keyrings/grafana.key] https://packages.grafana.com/oss/deb stable main" | sudo tee /etc/apt/sources.list.d/grafana.list
sudo apt-get update
sudo apt-get install grafana
sudo systemctl enable --now grafana-server
```

Accès : `http://votre-ip:3000` (admin/admin)

## Connexion VictoriaMetrics dans Grafana

1. **Configuration → Data Sources → Add data source**
2. Choisir **Prometheus**
3. URL : `http://192.168.1.141:8428`
4. Save & Test

## Dashboard PV recommandé (comparaison multi-années)

Créez un dashboard avec ces panels :

| Panel | Requête PromQL | Usage |
|-------|---------------|-------|
| **Production mensuelle** | `sum(increase(total_solar_power[30d]))` | Comparaison mois par mois |
| **Courbe annuelle** | `sum(total_solar_power)` sur 1y | Tendance annuelle |
| **Comparaison N vs N-1** | Deux queries superposées | Visuel immédiat |
| **Dégradation panneaux** | `sum(total_solar_power) / sum(victron_panel_power)` | Ratio efficacité |

## Génération PDF automatique

**Option 1 : Grafana natif (v9+)**
- Dashboard → Share → Export → PDF

**Option 2 : Grafana Image Renderer (recommandé)**
```bash
# Installer le plugin
grafana-cli plugins install grafana-image-renderer
sudo systemctl restart grafana-server
```
Puis : Dashboard → Share → Direct link rendered image → format PDF

**Option 3 : Rapports planifiés (si besoin)**
- API Grafana + cron pour générer un PDF mensuel automatiquement

## Requête PromQL utile pour comparaison annuelle

```promql
# Production totale de l'année en cours
sum(increase(total_solar_power[1y]))

# Production même période l'année dernière
sum(increase(total_solar_power[1y] offset 1y))

# Variation en %
(
  sum(increase(total_solar_power[1y])) 
  - sum(increase(total_solar_power[1y] offset 1y))
) 
/ sum(increase(total_solar_power[1y] offset 1y)) * 100
```

## Astuce : dashboard "Comparaison 5 ans"

Créez un dashboard avec **5 panels identiques**, chacun avec un `offset` différent :
- Panel 1 : `sum(total_solar_power)` (année en cours)
- Panel 2 : `sum(total_solar_power) offset 1y`
- Panel 3 : `sum(total_solar_power) offset 2y`
- etc.

Ou un seul graphique avec **5 courbes superposées** pour visualiser la dégradation.

---

Voulez-vous que je vous prépare un **dashboard Grafana JSON prêt à importer** avec ces panels de comparaison PV ?
