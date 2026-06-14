#!/usr/bin/env python3
# =============================================================================
# analyze-jeprof-diff.py — diff symbolisé de deux profils heap jemalloc.
#
# La liste plate de `jeprof --text` est ambiguë (LTO fusionne les symboles).
# Ce script agrège par PILE complète, calcule la croissance entre deux profils,
# et symbolise (addr2line) les piles qui ont le plus grossi → la vraie fuite,
# call-stack par call-stack.
#
# Usage :
#   python3 scripts/analyze-jeprof-diff.py <ancien.heap> <recent.heap> <binaire-symbolisé> [top_n]
#
# Le binaire symbolisé = target/aarch64-unknown-linux-gnu/release-symbols/daly-bms-server
# (issu de `make build-arm-symbols`, même .text que le binaire déployé).
# =============================================================================
import sys, subprocess, re

if len(sys.argv) < 4:
    print(__doc__); sys.exit(1)
old_path, new_path, binpath = sys.argv[1], sys.argv[2], sys.argv[3]
top_n = int(sys.argv[4]) if len(sys.argv) > 4 else 12

def parse(path):
    """Retourne (stacks: {tuple(addr_hex): bytes}, exe_base:int)."""
    stacks = {}
    base = None
    cur = None
    with open(path, errors="replace") as f:
        in_maps = False
        for line in f:
            if line.startswith("MAPPED_LIBRARIES"):
                in_maps = True
                continue
            if in_maps:
                # 5555e5210000-... r-xp 00000000 ... /usr/local/bin/daly-bms-server
                m = re.match(r"^([0-9a-f]+)-[0-9a-f]+ r-xp .*daly-bms-server\s*$", line)
                if m and base is None:
                    base = int(m.group(1), 16)
                continue
            s = line.strip()
            if line.startswith("@ "):
                cur = line[2:].split()
            elif s.startswith("t*:") and cur is not None:
                # "t*: <count>: <bytes> [..]"
                parts = s.split(":")
                try:
                    b = int(parts[2].strip().split()[0])
                except Exception:
                    cur = None; continue
                k = tuple(cur)
                stacks[k] = stacks.get(k, 0) + b
                cur = None
    return stacks, base

old, base_old = parse(old_path)
new, base_new = parse(new_path)
base = base_new or base_old
if base is None:
    print("!! base exécutable introuvable dans MAPPED_LIBRARIES"); sys.exit(2)

# Diff par pile
diff = {}
for k, b in new.items():
    diff[k] = diff.get(k, 0) + b
for k, b in old.items():
    diff[k] = diff.get(k, 0) - b
growing = sorted(((v, k) for k, v in diff.items() if v > 0), reverse=True)[:top_n]

# Adresses uniques à résoudre (seulement celles dans l'exécutable)
lo, hi = base, base + 0x10000000
addrs = set()
for _, k in growing:
    for a in k:
        ai = int(a, 16)
        if lo <= ai < hi:
            addrs.add(ai)
addrs = sorted(addrs)

# addr2line en batch
sym = {}
if addrs:
    offs = [hex(a - base) for a in addrs]
    try:
        out = subprocess.run(
            ["addr2line", "-f", "-C", "-e", binpath] + offs,
            capture_output=True, text=True, check=False).stdout.splitlines()
        # 2 lignes par adresse : fonction, fichier:ligne
        for i, a in enumerate(addrs):
            fn = out[2*i] if 2*i < len(out) else "??"
            loc = out[2*i+1] if 2*i+1 < len(out) else "??"
            loc = loc.split("/")[-1]
            sym[a] = f"{fn}  ({loc})"
    except FileNotFoundError:
        print("!! addr2line introuvable"); sys.exit(3)

def name(a):
    ai = int(a, 16)
    return sym.get(ai, a)

total_growth = sum(v for v, _ in growing)
print(f"# Top {len(growing)} piles ayant CRÛ — croissance cumulée {total_growth/1048576:.1f} Mo")
print(f"# binaire : {binpath}\n")
for v, k in growing:
    print(f"=== +{v/1048576:.2f} Mo " + "="*40)
    # pile = leaf -> ... ; on saute le préfixe allocateur (4 frames jemalloc)
    frames = list(k)
    # afficher du site d'allocation (après l'allocateur) vers la racine
    for a in frames[4:14]:
        print("   " + name(a))
    print()
