#!/bin/bash
# =============================================================================
# verify-no-loop.sh — Détecte les chevauchements OUT/IN dans mosquitto.conf
#
# Détecte :
#   1. Chaînes textuellement identiques (ex: OUT foo/bar + IN foo/bar)
#   2. Chevauchements via wildcards MQTT (`+` et `#`) :
#      - `foo/#` matche foo, foo/bar, foo/bar/baz
#      - `foo/+/baz` matche foo/X/baz
#      - Donc OUT `foo/bar` + IN `foo/#` = chevauchement = risque boucle
#
# Sortie : exit 0 si OK, exit 1 si chevauchement détecté.
# =============================================================================

set -euo pipefail

CONFIG="${1:-/etc/mosquitto/mosquitto.conf}"

if [ ! -f "$CONFIG" ]; then
    echo "ERREUR : $CONFIG introuvable" >&2
    exit 1
fi

command -v python3 >/dev/null 2>&1 || {
    echo "ERREUR : python3 requis pour le matching wildcard MQTT" >&2
    exit 1
}

python3 - "$CONFIG" << 'PYEOF'
import re, sys

def parse_topics(path):
    out, inc = [], []
    rx = re.compile(r'^\s*topic\s+(\S+)\s+(in|out|both)(?:\s+(\d+))?')
    with open(path) as f:
        for ln_no, line in enumerate(f, 1):
            s = line.strip()
            if not s or s.startswith('#'):
                continue
            m = rx.match(s)
            if not m:
                continue
            t, d = m.group(1), m.group(2)
            if d in ('out', 'both'):
                out.append((t, ln_no))
            if d in ('in', 'both'):
                inc.append((t, ln_no))
    return out, inc

def mqtt_match(pattern, topic):
    """Vrai si pattern MQTT (avec +, #) matche le topic concret."""
    if pattern == topic:
        return True
    p = pattern.split('/')
    t = topic.split('/')
    i = 0
    while i < len(p):
        seg = p[i]
        if seg == '#':
            return True
        if i >= len(t):
            return False
        if seg == '+':
            i += 1
            continue
        if seg != t[i]:
            return False
        i += 1
    return i == len(t)

def overlap(pat_a, pat_b):
    """Vrai si deux patterns MQTT peuvent matcher un même topic réel."""
    if pat_a == pat_b:
        return True
    a, b = pat_a.split('/'), pat_b.split('/')
    def concretize(p):
        return '/'.join('X' if s in ('+', '#') else s for s in p)
    if mqtt_match(pat_b, concretize(a)):
        return True
    if mqtt_match(pat_a, concretize(b)):
        return True
    # `#` peut matcher 0 segment : tester sans le dernier
    if a and a[-1] == '#' and len(a) > 1:
        if mqtt_match(pat_b, '/'.join(a[:-1])):
            return True
    if b and b[-1] == '#' and len(b) > 1:
        if mqtt_match(pat_a, '/'.join(b[:-1])):
            return True
    return False

path = sys.argv[1]
out, inc = parse_topics(path)

print("=== Topics EGRESS (out) ===")
for t, ln in sorted(out):
    print(f"  L{ln:<4} {t}")
print()
print("=== Topics INGRESS (in) ===")
for t, ln in sorted(inc):
    print(f"  L{ln:<4} {t}")
print()

print("=== Analyse des chevauchements (wildcard-aware) ===")
problems = []
for ot, oln in out:
    for it, iln in inc:
        if overlap(ot, it):
            problems.append((ot, oln, it, iln))

if problems:
    print()
    print("❌ DANGER — Chevauchements détectés (risque de BOUCLE) :")
    for ot, oln, it, iln in problems:
        kind = "IDENTIQUE" if ot == it else "WILDCARD "
        print(f"   {kind}  OUT(L{oln}) {ot}  ∩  IN(L{iln}) {it}")
    print()
    print("Pour fixer : resserrer le pattern IN pour exclure les topics OUT,")
    print("ou réorganiser les topics OUT pour éviter les wildcards IN trop larges.")
    sys.exit(1)
else:
    print("✅ OK : Aucun chevauchement OUT/IN. Pas de risque de boucle bridge.")
    sys.exit(0)
PYEOF
