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

# Diff par pile (peut être très fragmenté → on agrège ENSUITE par fonction)
diff = {}
for k, b in new.items():
    diff[k] = diff.get(k, 0) + b
for k, b in old.items():
    diff[k] = diff.get(k, 0) - b

# Toutes les adresses (dans l'exécutable) de toutes les piles à diff != 0
lo, hi = base, base + 0x10000000
addrs = set()
for k, v in diff.items():
    if v == 0:
        continue
    for a in k:
        ai = int(a, 16)
        if lo <= ai < hi:
            addrs.add(ai)
addrs = sorted(addrs)

# addr2line en batch (2 lignes par adresse : fonction, fichier:ligne)
sym = {}
if addrs:
    offs = [hex(a - base) for a in addrs]
    try:
        out = subprocess.run(
            ["addr2line", "-f", "-C", "-e", binpath] + offs,
            capture_output=True, text=True, check=False).stdout.splitlines()
    except FileNotFoundError:
        print("!! addr2line introuvable"); sys.exit(3)
    for i, a in enumerate(addrs):
        fn  = out[2*i]   if 2*i   < len(out) else "??"
        loc = out[2*i+1] if 2*i+1 < len(out) else "??"
        sym[a] = f"{fn}  ({loc.split('/')[-1]})"

def name(a):
    ai = int(a, 16)
    return sym.get(ai, a)

# Agrégation par FONCTION : pour chaque pile (diff != 0), on attribue son delta
# à chaque fonction UNIQUE de la pile (vue cumulée, comme jeprof). On ignore
# les 1res frames allocateur (jemalloc) communes à toutes les piles.
ALLOC_SKIP = 4
func_cum = {}
total = 0
for k, v in diff.items():
    if v == 0:
        continue
    total += v
    seen = set()
    for a in k[ALLOC_SKIP:]:
        f = name(a)
        if f in seen:
            continue
        seen.add(f)
        func_cum[f] = func_cum.get(f, 0) + v

ranked = sorted(func_cum.items(), key=lambda x: x[1], reverse=True)
print(f"# Croissance nette totale : {total/1048576:.1f} Mo")
print(f"# Top fonctions par octets cumulés sur les chemins qui ont CRÛ :\n")
for f, b in ranked[:35]:
    if b <= 0:
        continue
    print(f"{b/1048576:7.2f} Mo  {f}")
