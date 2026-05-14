# 📊 Perses + VictoriaMetrics pour Monitoring PV Solaire

Guide d'installation de **Perses** (alternative légère et GitOps à Grafana) sur Raspberry Pi 5 avec migration du dashboard PV Solaire.

---

## 🚀 Installation (natif - recommandée sur Pi 5)

Perses est très léger (écrit en Go) et idéal pour un Raspberry Pi 5.

### 1. Téléchargement du binaire ARM64

```bash
# Créer un dossier dédié
mkdir -p ~/perses && cd ~/perses

# Récupérer la dernière version (remplace par la version actuelle)
VERSION=$(curl -s https://api.github.com/repos/perses/perses/releases/latest | grep tag_name | cut -d '"' -f 4)

wget https://github.com/perses/perses/releases/download/${VERSION}/perses_${VERSION#v}_linux_arm64.tar.gz

tar xzf perses_${VERSION#v}_linux_arm64.tar.gz
sudo cp perses /usr/local/bin/
sudo cp percli /usr/local/bin/
2. Configuration minimale
Crée le fichier /etc/perses/config.yaml :
# /etc/perses/config.yaml
server:
  port: 8090
  enableUI: true

database:
  file:
    folder: /var/lib/perses/db
    extension: json

provisioning:
  dashboards:
    - folder: /etc/perses/dashboards
  datasources:
    - folder: /etc/perses/datasources
Crée les dossiers :
sudo mkdir -p /etc/perses/{dashboards,datasources} /var/lib/perses/db
sudo chown -R $USER:$USER /etc/perses /var/lib/perses
3. Service systemd
sudo tee /etc/systemd/system/perses.service > /dev/null <
4. Accès
	•	URL : http://:8080
	•	Premier accès : pas de login par défaut (tu peux configurer l’authentification plus tard).

🔌 Configuration Data Source VictoriaMetrics
Crée /etc/perses/datasources/victoriametrics.yaml :
apiVersion: 1
kind: Datasource
metadata:
  name: victoriametrics
  project: perses
spec:
  display:
    name: VictoriaMetrics
  plugin:
    kind: Prometheus
    spec:
      url: http://127.0.0.1:8428
      proxy: true
      timeInterval: 10s
Redémarre Perses :
sudo systemctl restart perses

📈 Migration du Dashboard PV Solaire
Méthode recommandée : via percli
# Migration du dashboard Grafana
percli migrate -f pv-solar-5y.json --online -o yaml > perses-pv-solar-5y.yaml
Puis applique-le :
percli apply -f perses-pv-solar-5y.yaml --project perses
Alternative via UI :
	1	Va dans Dashboards → Create → Import / Migrate.
	2	Colle ou uploade ton JSON Grafana.
Note : La migration est best-effort. Les panels Prometheus fonctionnent très bien ; certains styles ou panels avancés peuvent nécessiter des ajustements manuels.
Copie le dashboard migré dans /etc/perses/dashboards/ pour du provisioning automatique.

🔧 Requêtes PromQL (adaptées)
Perses utilise le même langage PromQL / MetricsQL que Grafana.
Utilise les métriques de ton projet (solar_total_w, solar_yield_kwh, etc.) comme dans ton dashboard Grafana.

📄 Export PDF / Rapports
Perses ne possède pas encore d’export PDF natif aussi mature que Grafana, mais tu peux :
	1	Utiliser l’export image du navigateur.
	2	Installer un outil externe (wkhtmltopdf, puppeteer, ou un script avec Playwright).
	3	Ou garder Grafana en parallèle pour les exports PDF pendant la phase de test.

📋 Architecture Recommandée
Victron GX → MQTT → Pi5 (Daly-BMS) → VictoriaMetrics
                                           ↓
                                    Perses (port 8090)
Avantages sur Pi 5 :
	•	Empreinte mémoire beaucoup plus faible que Grafana.
	•	Dashboards en fichiers YAML (versionnables dans Git).
	•	Parfait pour du GitOps.

📝 Notes & Optimisations
	•	Ports : Grafana (3000) + Perses (8090) peuvent tourner en parallèle sans problème.
	•	Reverse Proxy : Utilise Nginx pour accéder via https://perses.mondomaine.local.
	•	Mise à jour : Télécharge la nouvelle version et remplace le binaire.
	•	Sécurité : Configure l’authentification (OIDC, basic auth) dans config.yaml pour la production.
	•	Rétention : Gérée côté VictoriaMetrics (-retentionPeriod=5y).
Pour tester en parallèle : Laisse Grafana actif pendant que tu valides Perses.

Guide créé pour une migration progressive Grafana → Perses sur Raspberry Pi 5 avec monitoring PV Victron + VictoriaMetrics.
