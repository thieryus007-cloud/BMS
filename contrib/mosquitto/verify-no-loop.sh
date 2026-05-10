#!/bin/bash
# verify-no-loop.sh
# Vérifie qu'aucun topic n'est présent à la fois en IN et en OUT

CONFIG="/etc/mosquitto/mosquitto.conf"

if [ ! -f "$CONFIG" ]; then
    echo "ERREUR : $CONFIG introuvable"
    exit 1
fi

echo "=== Topics EGRESS (out) ==="
OUT_TOPICS=$(grep -E '^\s*topic\s+\S+\s+out\s' "$CONFIG" | awk '{print $2}' | sort -u)
echo "$OUT_TOPICS"

echo ""
echo "=== Topics INGRESS (in) ==="
IN_TOPICS=$(grep -E '^\s*topic\s+\S+\s+in\s' "$CONFIG" | awk '{print $2}' | sort -u)
echo "$IN_TOPICS"

echo ""
echo "=== INTERSECTION (DANGER — topics en double) ==="
INTERSECTION=$(comm -12 <(echo "$OUT_TOPICS") <(echo "$IN_TOPICS"))

if [ -n "$INTERSECTION" ]; then
    echo "❌ ERREUR FATALE : Topics présents dans les deux directions :"
    echo "$INTERSECTION"
    echo ""
    echo "Cela créera une BOUCLE INFINIE. Corriger mosquitto.conf immédiatement."
    exit 1
else
    echo "✅ OK : Aucun topic en double. Pas de risque de boucle."
    exit 0
fi
