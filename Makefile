# =============================================================================
# DalyBMS — Rust Edition — Makefile (Optimisé Pi5)
# =============================================================================
# Usage :
# make build → compiler en release (x86_64)
# make build-arm → compiler pour aarch64 (Raspberry Pi CM5 / NanoPi) [OPTIMISÉ]
# make build-arm-debug → compiler avec symboles pour profiling (perf/flamegraph)
# make run → lancer le serveur en dev
# make test → lancer les tests unitaires
# make install → installer le binaire et le service systemd
# make lint → clippy + fmt check

CARGO := cargo
BINARY := daly-bms-server
ENERGY_BIN := energy-manager
TARGET_ARM := aarch64-unknown-linux-gnu
TARGET_ARMV7 := armv7-unknown-linux-gnueabihf
TARGET_MUSL := aarch64-unknown-linux-musl
RELEASE_DIR := target/release
ARM_RELEASE_DIR := target/$(TARGET_ARM)/release
ARM_DEBUG_DIR   := target/$(TARGET_ARM)/release-debug
ARMV7_RELEASE_DIR := target/$(TARGET_ARMV7)/release
MUSL_RELEASE_DIR := target/$(TARGET_MUSL)/release

# =============================================================================
# Flags d'optimisation pour Raspberry Pi 5 (Cortex-A76 + NEON)
# =============================================================================
# target-cpu=native : utilise les instructions spécifiques du CPU hôte
# link-arg=-Wl,--as-needed : réduit la taille du binaire
# link-arg=-Wl,--strip-all : supprime les symboles de debug (sauf pour debug build)
ARM_RUSTFLAGS := -C target-cpu=native -C link-arg=-Wl,--as-needed
ARM_RUSTFLAGS_DEBUG := $(ARM_RUSTFLAGS) -C force-frame-pointers=yes -C debuginfo=2
ARM_RUSTFLAGS_MUSL := -C target-cpu=native -C link-arg=-Wl,--strip-all -C link-arg=-Wl,--as-needed

# Linkers cross-compilation
CROSS_LINKER_GNU := aarch64-linux-gnu-gcc
CROSS_LINKER_MUSL := aarch64-linux-musl-gcc
CROSS_LINKER_ARMV7 := arm-linux-gnueabihf-gcc

# =============================================================================
# Broker MQTT — Mosquitto natif sur Pi5 (remplace Docker)
# =============================================================================
# Service systemd : mosquitto-broker.service
# Config          : /etc/mosquitto/mosquitto.conf (source : contrib/mosquitto/)
# Bascule Docker → natif : contrib/mosquitto/deploy-mosquitto-native.sh
# Nettoyage post-bascule : contrib/mosquitto/cleanup-docker.sh
#
# Commandes utiles :
#   sudo systemctl {start,stop,restart,status} mosquitto-broker
#   journalctl -u mosquitto-broker -f
#   sudo /usr/local/bin/verify-no-loop.sh   # vérifie l'absence de boucle bridge

# =============================================================================
# Vérification des dépendances cross-compilation
# =============================================================================

.PHONY: check-arm-deps check-armv7-deps check-musl-deps

check-arm-deps:
	@echo "🔍 Vérification dépendances cross-compilation ARM64 (gnu)..."
	@rustup target list --installed | grep -q $(TARGET_ARM) || \
		(echo "→ Ajout cible $(TARGET_ARM)..." && rustup target add $(TARGET_ARM))
	@which $(CROSS_LINKER_GNU) >/dev/null 2>&1 || \
		(echo "❌ $(CROSS_LINKER_GNU) manquant. Installer:" && \
		 echo "  Debian/Ubuntu: sudo apt install gcc-aarch64-linux-gnu" && \
		 echo "  Fedora: sudo dnf install gcc-aarch64-linux-gnu" && \
		 exit 1)
	@echo "✓ Dépendances ARM64-gnu OK"

check-armv7-deps:
	@echo "🔍 Vérification dépendances cross-compilation ARMv7..."
	@rustup target list --installed | grep -q $(TARGET_ARMV7) || \
		(echo "→ Ajout cible $(TARGET_ARMV7)..." && rustup target add $(TARGET_ARMV7))
	@which $(CROSS_LINKER_ARMV7) >/dev/null 2>&1 || \
		(echo "❌ $(CROSS_LINKER_ARMV7) manquant. Installer:" && \
		 echo "  Debian/Ubuntu: sudo apt install gcc-arm-linux-gnueabihf" && \
		 exit 1)
	@echo "✓ Dépendances ARMv7 OK"

check-musl-deps:
	@echo "🔍 Vérification dépendances cross-compilation ARM64 (musl)..."
	@rustup target list --installed | grep -q $(TARGET_MUSL) || \
		(echo "→ Ajout cible $(TARGET_MUSL)..." && rustup target add $(TARGET_MUSL))
	@which $(CROSS_LINKER_MUSL) >/dev/null 2>&1 || \
		(echo "❌ $(CROSS_LINKER_MUSL) manquant. Installer:" && \
		 echo "  Debian/Ubuntu: sudo apt install musl-tools aarch64-linux-gnu" && \
		 exit 1)
	@echo "✓ Dépendances ARM64-musl OK"

# =============================================================================
# Compilation — Version optimisée
# =============================================================================

.PHONY: build build-arm build-arm-debug build-arm-musl build-arm-v7 build-venus build-venus-arm build-venus-armv7 build-venus-v7 build-energy build-energy-arm install-energy run-energy

VENUS_BIN := dbus-mqtt-venus

# Build natif (x86_64 ou architecture hôte)
build:
	$(CARGO) build --release --bin $(BINARY)
	@echo "✓ Binaire : $(RELEASE_DIR)/$(BINARY)"

# Build ARM64 optimisé pour Pi5 (RECOMMANDÉ — Cortex-A76 + NEON)
build-arm: check-arm-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(CROSS_LINKER_GNU) \
	RUSTFLAGS="$(ARM_RUSTFLAGS)" \
	$(CARGO) build --release --target $(TARGET_ARM) --bin $(BINARY)
	@echo "✓ Binaire ARM optimisé Pi5 : $(ARM_RELEASE_DIR)/$(BINARY)"
	@ls -lh $(ARM_RELEASE_DIR)/$(BINARY) 2>/dev/null || true

# Build ARM64 avec symboles pour profiling (perf/flamegraph)
build-arm-debug: check-arm-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(CROSS_LINKER_GNU) \
	RUSTFLAGS="$(ARM_RUSTFLAGS_DEBUG)" \
	$(CARGO) build --profile release-debug --target $(TARGET_ARM) --bin $(BINARY)
	@echo "✓ Binaire ARM avec symboles : $(ARM_DEBUG_DIR)/$(BINARY)"
	@echo "  → Profiler sur Pi5 : sudo perf record -F 99 -g ./$(BINARY) && sudo perf report"

# Build ARM64 statique (musl) — plus portable, binaire autonome
build-arm-musl: check-musl-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=$(CROSS_LINKER_MUSL) \
	RUSTFLAGS="$(ARM_RUSTFLAGS_MUSL)" \
	$(CARGO) build --release --target $(TARGET_MUSL) --bin $(BINARY)
	@echo "✓ Binaire ARM statique (musl) : $(MUSL_RELEASE_DIR)/$(BINARY)"
	@ls -lh $(MUSL_RELEASE_DIR)/$(BINARY) 2>/dev/null || true

# Phase 3 — Venus OS D-Bus bridge
build-venus:
	$(CARGO) build --release --bin $(VENUS_BIN)
	@echo "✓ Binaire Venus : $(RELEASE_DIR)/$(VENUS_BIN)"

build-venus-arm: check-arm-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(CROSS_LINKER_GNU) \
	RUSTFLAGS="$(ARM_RUSTFLAGS)" \
	$(CARGO) build --release --target $(TARGET_ARM) --bin $(VENUS_BIN) --bin $(BINARY)
	@echo "✓ Binaires ARM Venus OS (optimisés Pi5) :"
	@echo "  $(ARM_RELEASE_DIR)/$(BINARY)"
	@echo "  $(ARM_RELEASE_DIR)/$(VENUS_BIN)"

build-arm-v7: check-armv7-deps
	CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=$(CROSS_LINKER_ARMV7) \
	RUSTFLAGS="-C target-cpu=native -C link-arg=-Wl,--as-needed" \
	$(CARGO) build --release --target $(TARGET_ARMV7) --bin $(BINARY)
	@echo "✓ Binaire ARMv7 : $(ARMV7_RELEASE_DIR)/$(BINARY)"

build-venus-armv7 build-venus-v7: check-armv7-deps
	CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=$(CROSS_LINKER_ARMV7) \
	RUSTFLAGS="-C target-cpu=native -C link-arg=-Wl,--as-needed" \
	$(CARGO) build --release --target $(TARGET_ARMV7) --bin $(VENUS_BIN) --bin $(BINARY)
	@echo "✓ Binaires ARMv7 Venus OS :"
	@echo "  $(ARMV7_RELEASE_DIR)/$(BINARY)"
	@echo "  $(ARMV7_RELEASE_DIR)/$(VENUS_BIN)"

# Déploiement sur Venus OS (remplacer GX_IP par l'IP de votre GX)
GX_IP ?= 192.168.1.120
install-venus: build-venus-arm
	./nanoPi/install-venus.sh $(GX_IP)

# Déploiement sur Venus OS armv7l (NanoPi 32-bit)
install-venus-v7: build-venus-armv7
	ARCH=armv7 ./nanoPi/install-venus.sh $(GX_IP)

build-energy:
	$(CARGO) build --release --bin $(ENERGY_BIN)
	@echo "✓ Binaire : $(RELEASE_DIR)/$(ENERGY_BIN)"

build-energy-arm: check-arm-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(CROSS_LINKER_GNU) \
	RUSTFLAGS="$(ARM_RUSTFLAGS)" \
	$(CARGO) build --release --target $(TARGET_ARM) --bin $(ENERGY_BIN)
	@echo "✓ Binaire ARM energy-manager (optimisé Pi5) : $(ARM_RELEASE_DIR)/$(ENERGY_BIN)"

# Migration redb : binaire d'import VM → redb (cf. docs/plan_migration_vm_redb.md §0.7)
build-import-vm-arm: check-arm-deps
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$(CROSS_LINKER_GNU) \
	RUSTFLAGS="$(ARM_RUSTFLAGS)" \
	$(CARGO) build --release --target $(TARGET_ARM) --bin import-vm
	@echo "✓ Binaire ARM import-vm (optimisé Pi5) : $(ARM_RELEASE_DIR)/import-vm"

install-energy: build-energy-arm
	scp $(ARM_RELEASE_DIR)/$(ENERGY_BIN) $(PI_HOST):/tmp/$(ENERGY_BIN)
	ssh $(PI_HOST) "sudo install -m 755 /tmp/$(ENERGY_BIN) /usr/local/bin/$(ENERGY_BIN) && sudo systemctl restart energy-manager && sudo systemctl status energy-manager --no-pager -l"
	@echo "✓ energy-manager déployé sur $(PI_HOST)"

run-energy:
	RUST_LOG=info $(CARGO) run --release --bin $(ENERGY_BIN)

build-all:
	$(CARGO) build --release

# =============================================================================
# Développement
# =============================================================================

.PHONY: run run-debug

run:
	RUST_LOG=info $(CARGO) run --release --bin $(BINARY)

run-debug:
	RUST_LOG=debug $(CARGO) run --bin $(BINARY)

# =============================================================================
# Tests
# =============================================================================

.PHONY: test test-core test-verbose

test:
	$(CARGO) test --workspace

test-core:
	$(CARGO) test -p daly-bms-core

test-verbose:
	$(CARGO) test --workspace -- --nocapture

# =============================================================================
# Qualité
# =============================================================================

.PHONY: lint fmt check

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

check:
	$(CARGO) check --workspace
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace

# =============================================================================
# Installation (systemd)
# =============================================================================

.PHONY: install uninstall install-z8run uninstall-node-exporter

install: build
	sudo bash contrib/install-systemd.sh

uninstall:
	sudo bash contrib/uninstall-systemd.sh

install-z8run:
	sudo bash contrib/install-z8run.sh

# Désinstalle node_exporter du Pi5 (retiré du projet — les métriques OS sont
# désormais collectées directement par monitor.rs dans daly-bms-server).
# Usage : make uninstall-node-exporter  (à exécuter depuis le Pi5)
uninstall-node-exporter:
	-sudo systemctl disable --now node-exporter 2>/dev/null
	-sudo rm -f /etc/systemd/system/node-exporter.service
	-sudo rm -f /usr/local/bin/node_exporter
	sudo systemctl daemon-reload
	@echo "✓ node_exporter désinstallé. Pense à retirer le job 'node' de /etc/victoriametrics/scrape.yml"

# =============================================================================
# Perses — Dashboard monitoring (essai parallèle à Grafana)
# =============================================================================

.PHONY: perses-install perses-uninstall

# Installe Perses sur le Pi5 (exécuter directement sur le Pi5)
# Options : make perses-install PERSES_ARGS="--nvme"
PERSES_ARGS ?= --nvme

perses-install:
	bash scripts/setup-perses.sh $(PERSES_ARGS)

perses-uninstall:
	sudo bash scripts/setup-perses.sh --uninstall

# =============================================================================
# Cross-compile + déploiement SSH vers le Pi
# =============================================================================

PI_HOST ?= pi5compute@192.168.1.141
PI_BIN_PATH ?= /usr/local/bin/daly-bms-server
BRANCH ?= $(shell git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)

# sync : utiliser sur Pi5 à la place de git pull
# Écrase les fichiers locaux sans créer de commits.
# Corrige aussi tout fichier accidentellement détenu par root avant reset.
.PHONY: sync
sync:
	git fetch origin $(BRANCH)
	@sudo chown -R $(shell whoami):$(shell whoami) . 2>/dev/null || true
	git reset --hard origin/$(BRANCH)
	@echo "✓ Synchronisé sur origin/$(BRANCH) (aucun commit local)"

.PHONY: deploy deploy-musl

# Déploiement standard (binaire gnu dynamique)
deploy: build-arm
	scp $(ARM_RELEASE_DIR)/$(BINARY) $(PI_HOST):/tmp/$(BINARY)
	ssh $(PI_HOST) "sudo install -m 755 /tmp/$(BINARY) $(PI_BIN_PATH) && sudo systemctl restart daly-bms && sudo systemctl status daly-bms --no-pager -l"
	@echo "✓ Déployé sur $(PI_HOST) (binaire: $(ARM_RELEASE_DIR)/$(BINARY))"

# Déploiement avec binaire musl statique (plus portable)
deploy-musl: build-arm-musl
	scp $(MUSL_RELEASE_DIR)/$(BINARY) $(PI_HOST):/tmp/$(BINARY)
	ssh $(PI_HOST) "sudo install -m 755 /tmp/$(BINARY) $(PI_BIN_PATH) && sudo systemctl restart daly-bms && sudo systemctl status daly-bms --no-pager -l"
	@echo "✓ Déployé sur $(PI_HOST) (binaire statique musl: $(MUSL_RELEASE_DIR)/$(BINARY))"

# =============================================================================
# Profiling & Diagnostic (nouveaux targets)
# =============================================================================

.PHONY: profile-setup profile-start profile-stop profile-report

# Installer perf sur le Pi5 (à exécuter via ssh ou directement sur Pi5)
profile-setup:
	@echo "🔧 Pour profiler sur le Pi5, exécutez MANUELLEMENT sur le Pi5 :"
	@echo "  sudo apt update && sudo apt install -y linux-perf"
	@echo "  sudo sysctl -w kernel.perf_event_paranoid=1"
	@echo "  sudo sysctl -w kernel.kptr_restrict=0"

# Démarrer l'enregistrement perf (sur Pi5)
profile-start:
	@echo "📊 Démarrage profiling (30 secondes)..."
	@echo "Exécutez sur le Pi5 :"
	@echo "  sudo perf record -F 99 -p \$$(pgrep daly-bms-server) -g -- sleep 30"

# Arrêter perf et générer rapport
profile-stop:
	@echo "📈 Pour analyser les résultats, sur le Pi5 :"
	@echo "  sudo perf report --stdio | head -50"
	@echo "  # Ou pour flamegraph :"
	@echo "  sudo perf script | stackcollapse-perf.pl | flamegraph.pl > flame.svg"

# =============================================================================
# Dashboard (React)
# =============================================================================

.PHONY: dashboard-dev dashboard-build

dashboard-dev:
	cd dashboard && npm run dev

dashboard-build:
	cd dashboard && npm run build

# =============================================================================
# Documentation
# =============================================================================

.PHONY: doc

doc:
	$(CARGO) doc --workspace --no-deps --open

# =============================================================================
# Nettoyage
# =============================================================================

.PHONY: clean

clean:
	$(CARGO) clean

.DEFAULT_GOAL := help

.PHONY: help
help:
	@echo ""
	@echo "DalyBMS Rust Edition — Commandes disponibles :"
	@echo ""
	@echo " Broker MQTT (Mosquitto natif systemd, plus de Docker) :"
	@echo "  sudo systemctl status mosquitto-broker"
	@echo "  journalctl -u mosquitto-broker -f"
	@echo ""
	@echo " Compilation :"
	@echo "  make build           Compiler pour l'architecture locale"
	@echo "  make build-arm       Cross-compiler pour aarch64 [OPTIMISÉ Pi5] ⭐"
	@echo "  make build-arm-debug Compiler avec symboles pour profiling"
	@echo "  make build-arm-musl  Compiler binaire statique portable (musl)"
	@echo "  make build-arm-v7    Compiler pour ARMv7 (NanoPi 32-bit)"
	@echo "  make build-all       Compiler tous les binaires"
	@echo ""
	@echo " Développement :"
	@echo "  make run             Lancer le serveur (release)"
	@echo "  make run-debug       Lancer en mode debug (RUST_LOG=debug)"
	@echo ""
	@echo " Tests & Qualité :"
	@echo "  make test            Tests unitaires"
	@echo "  make lint            Clippy"
	@echo "  make fmt             Format code"
	@echo "  make check           Check + fmt + lint"
	@echo ""
	@echo " Déploiement :"
	@echo "  make install         Installer le service systemd daly-bms"
	@echo "  make install-z8run   Installer z8run (build + service systemd, sur Pi5)"
	@echo "  make deploy          Déployer sur pi5compute@192.168.1.141 [gnu]"
	@echo "  make deploy-musl     Déployer binaire statique portable [musl]"
	@echo "  make install-venus   Déployer dbus-mqtt-venus sur GX (ARM64)"
	@echo "  make install-venus-v7 Déployer sur NanoPi (armv7)"
	@echo ""
	@echo " Perses (monitoring — essai parallèle à Grafana) :"
	@echo "  make perses-install  Installer Perses sur le Pi5 (port 8090)"
	@echo "  make perses-uninstall Désinstaller Perses"
	@echo "  Voir docs/Perses-readme.md pour le guide complet"
	@echo ""
	@echo " Profiling (optimisation CPU) :"
	@echo "  make profile-setup   Instructions pour installer perf sur Pi5"
	@echo "  make profile-start   Démarrer l'enregistrement perf"
	@echo "  make profile-report  Générer rapport/flamegraph"
	@echo ""
	@echo " 💡 Conseil Pi5 : Utilisez 'make build-arm' pour des performances optimales"
	@echo "    (active target-cpu=native pour Cortex-A76 + NEON)"
	@echo ""
