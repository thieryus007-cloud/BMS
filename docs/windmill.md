Voici un processus simple et adapté pour installer Windmill sur ton Raspberry Pi 5 avec Docker, tout en gardant ton Mosquitto existant.
1. Prérequis
	•	Docker et Docker Compose déjà installés (si ce n’est pas le cas : curl -fsSL https://get.docker.com | sudo sh puis sudo usermod -aG docker $USER et reconnecte-toi).
	•	Ton conteneur Mosquitto tourne déjà (il utilise probablement le port 1883).
2. Installation de Windmill
Crée un dossier dédié :
mkdir ~/windmill && cd ~/windmill
Récupère les fichiers officiels :
curl https://raw.githubusercontent.com/windmill-labs/windmill/main/docker-compose.yml -o docker-compose.yml
curl https://raw.githubusercontent.com/windmill-labs/windmill/main/Caddyfile -o Caddyfile
curl https://raw.githubusercontent.com/windmill-labs/windmill/main/.env -o .env
3. Adaptations importantes pour Raspberry Pi 5 + Mosquitto
Édite le fichier .env :
nano .env
Modifie ou ajoute :
WM_IMAGE=ghcr.io/windmill-labs/windmill:main
DATABASE_URL=postgres://postgres:changeme@db/windmill?sslmode=disable

# Change le port si tu veux (Caddy écoute par défaut sur 80)
# BASE_URL=":80"
Édite le docker-compose.yml pour éviter les conflits de ports avec Mosquitto et optimiser pour le Pi 5 :
	•	Réduis les ressources (le Pi 5 n’est pas un monstre) → limite les workers.
	•	Désactive Caddy sur le port 80 si tu utilises déjà un reverse proxy, ou change le port exposé.
	•	Ajoute une contrainte pour forcer arm64 si besoin.
Exemple de modifications minimales :
# Dans windmill_worker et windmill_worker_native :
deploy:
  replicas: 1          # Réduis à 1 ou 2 au lieu de 3
  resources:
    limits:
      memory: 1024M    # ou 1536M max selon ta RAM

# Dans le service caddy (ports) :
ports:
  - "8080:80"          # Change 80:80 → 8080:80 pour éviter conflit si tu as un autre service web
  # - "25:25"          # SMTP si tu en as besoin
Pour le problème jemalloc / page size sur Pi 5 (16K pages) :
Ajoute dans la section environment des services windmill_server, windmill_worker, etc. :
environment:
  - DISABLE_JEMALLOC=true   # Si ça ne marche pas, essaie aussi LD_PRELOAD=""
Ou (solution qui a marché pour plusieurs utilisateurs) :
Ajoute à la fin de /boot/firmware/config.txt :
kernel=kernel8.img
Puis redémarre le Pi.
4. Lancement
docker compose up -d
Attends 1-2 minutes puis va sur http://IP_DE_TON_PI (ou :8080 si tu as changé le port).
5. Accès initial
	•	Crée ton compte admin lors du premier accès.
	•	Va dans Instance settings → ajuste les ressources workers si besoin.

mkdir ~/windmill && cd ~/windmill

curl https://raw.githubusercontent.com/windmill-labs/windmill/main/docker-compose.yml -o docker-compose.yml
curl https://raw.githubusercontent.com/windmill-labs/windmill/main/Caddyfile -o Caddyfile
curl https://raw.githubusercontent.com/windmill-labs/windmill/main/.env -o .env

fichier .env

WM_IMAGE=ghcr.io/windmill-labs/windmill:main
DATABASE_URL=postgres://postgres:changeme@db/windmill?sslmode=disable

version: "3.7"

x-logging: &default-logging
  driver: "json-file"
  options:
    max-size: "10m"
    max-file: "5"

services:
  db:
    image: postgres:16
    restart: unless-stopped
    volumes:
      - db_data:/var/lib/postgresql/data
    expose:
      - 5432
    environment:
      POSTGRES_PASSWORD: changeme
      POSTGRES_DB: windmill
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 10s
      timeout: 5s
      retries: 5
    logging: *default-logging

  windmill_server:
    image: ${WM_IMAGE}
    pull_policy: always
    restart: unless-stopped
    expose:
      - 8000
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - MODE=server
      - DISABLE_JEMALLOC=true   # Important pour Pi 5
    depends_on:
      db:
        condition: service_healthy
    logging: *default-logging

  windmill_worker:
    image: ${WM_IMAGE}
    pull_policy: always
    restart: unless-stopped
    privileged: true
    deploy:
      replicas: 1                    # Très important sur Pi 5
      resources:
        limits:
          memory: 800M               # Ajuste selon ta RAM (4GB ou 8GB)
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - MODE=worker
      - WORKER_GROUP=default
      - FAVOR_UNSHARE_PID=true
      - DISABLE_JEMALLOC=true        # Important pour Pi 5
      # Optionnel : désactive dind si tu n'en as pas besoin
      # - DOCKER_HOST=tcp://dind:2375
    depends_on:
      db:
        condition: service_healthy
    volumes:
      - worker_dependency_cache:/tmp/windmill/cache
      - worker_logs:/tmp/windmill/logs
    logging: *default-logging

  windmill_worker_native:
    image: ${WM_IMAGE}
    pull_policy: always
    restart: unless-stopped
    deploy:
      replicas: 1
      resources:
        limits:
          memory: 400M
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - MODE=worker
      - WORKER_GROUP=native
      - NATIVE_MODE=true
      - DISABLE_JEMALLOC=true
    depends_on:
      db:
        condition: service_healthy
    volumes:
      - worker_logs:/tmp/windmill/logs
    logging: *default-logging

  caddy:
    image: ghcr.io/windmill-labs/caddy-l4:latest
    restart: unless-stopped
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
    ports:
      - "8080:80"          # Changé pour éviter conflit avec d'autres services web
      # - "25:25"          # SMTP si besoin
    environment:
      - BASE_URL=":80"
    logging: *default-logging

volumes:
  db_data:
  worker_dependency_cache:
  worker_logs:
  caddy_data:
