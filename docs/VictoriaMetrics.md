## Diagnostic VMUI — copier-coller direct

**1. Voir si VM contient quelque chose :**
```
{__name__!=""}
```

**2. Via l'API — liste de toutes les métriques reçues :**
```
http://192.168.1.141:8428/api/v1/label/__name__/values
```

---

## Requêtes par source de données

| Source | Requête VMUI |
|--------|-------------|
| BMS-360Ah SOC | `bms_soc{bms_id="0x01"}` |
| BMS-360Ah courant | `bms_current{bms_id="0x01"}` |
| ET112 micro-onduleurs | `et112_power_w{address="0x07"}` |
| ET112 maison | `et112_power_w{address="0x08"}` |
| ET112 réseau | `et112_power_w{address="0x09"}` |
| **SmartShunt courant** | `venus_shunt_current_a` |
| **SmartShunt puissance** | `venus_shunt_power_w` |
| Onduleur DC | `venus_inverter_power_w` |
| Solaire total | `solar_total_w` |
| Irradiance | `irradiance_wm2` |

---

## Vérifier les écritures (sur Pi5)

```bash
# Compter les rows reçues par VM depuis le démarrage
curl -s "http://127.0.0.1:8428/metrics" | grep "vm_rows_received_total"

# Tester une écriture manuelle vers VM
curl -X POST "http://127.0.0.1:8428/api/v1/import/prometheus" \
  --data 'test_metric{source="manual"} 42.0'

# Vérifier que le test est arrivé
curl -s "http://127.0.0.1:8428/api/v1/query?query=test_metric"
```
