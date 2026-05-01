## Appareils et Métriques:
3 x Daly BMS 16 cellules
- Métriques par BMS :
- Tension par cellule (16)
- Delta cellule mV
- Température par BMS (2)
- Courant de charge/décharge (1)
- Tension totale
- État de santé (SOC, SOH, etc.) (2)
Total par BMS : ~40 métriques
Total pour 3 BMS : \( 40 imes 3 = 120 \) séries temporelles

##  3 x ET112 Energy Management
- Métriques par ET112 :
- Puissance active (1)
- Puissance réactive (1)
- Tension (1)
- Courant (1)
- Importée
- Exportée
Total par ET112 : ~6 métriques
Total pour 3 ET112 : \( 6 imes 3 = 18 \) séries temporelles

## 1 x Capteur Irradiance
- Métriques :
- Irradiance (1)
Total : 1 série temporelle

## 1 x ATS (Automatic Transfer Switch)
- Métriques :
- État (ON/OFF) (3)
- Courant (3)
- Tension (3)
- xxx
Total : 10 séries temporelles

## 2 x MPPT Victron
- Métriques par MPPT :
- Puissance (1)
- Tension d'entrée (1)
- Courant d'entrée (1)
- Tension de sortie (1)
- Courant de sortie (1)
Total par MPPT : ~5 métriques
Total pour 2 MPPT : \( 2 imes 5 = 10 \) séries temporelles

## 1 x SmartShunt Victron
- Métriques :
- Courant (1)
- Tension (1)
- SOC (1)
- Puissance (1)
- Energie chargée
- Energie déchargée
Total : 6 séries temporelles

## 1 x Easysolar II GX Victron
- Métriques :
- Puissance totale (3)
- Tension (3)
- Courant (3)
- Température (1)
- Ingnore AC IN
Total : 8 séries temporelles

## 1 x Capteur Température + Humidité
- Métriques :
- Température (1)
- Humidité (1)
Total : 2 séries temporelles

## 6 x Switchs Tasmota Tonguou
- Métriques par switch :
- État (ON/OFF) (1)
- Puissance (1)
- Tension
- Courant
- Jours Énergie totale kWh(1)
Total par switch : ~5 métriques
Total pour 6 switchs : \( 5 imes 6 = 30 \) séries temporelles

## 1 x Switch Shelly Pro 2 PM
- Métriques :
- État (ON/OFF) (2)
- Puissance (2)
- Énergie totale (2)
Total : 6 séries temporelles

## Total des Séries Temporelles
| Appareil | Séries Temporelles |
|------------------------------|--------------------|
| 3 x Daly BMS 16 cellules | 105 |
| 3 x ET112 Energy Management | 12 |
| 1 x Capteur Irradiance | 1 |
| 1 x ATS | 10 |
| 2 x MPPT Victron | 10 |
| 1 x SmartShunt Victron | 6 |
| 1 x Easysolar II GX Victron | 8 |
| 1 x Capteur Température/Humidité | 2 |
| 6 x Switchs Tasmota Tonguou | 30 |
| 1 x Switch Shelly Pro 2 PM | 6|
| Total | ~222 |

Conclusion:
Nombre mini/maximal de séries temporelles : ~230/240 (en incluant des métriques supplémentaires ou des tags supplémentaires dans VictoriaMetrics).
