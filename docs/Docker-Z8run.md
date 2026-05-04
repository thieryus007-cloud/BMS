```markdown
# Installation de Z8RUN sur Raspberry Pi 5 avec Docker et SQLite

Ce guide vous permet d’installer **Z8RUN** (plateforme IoT/Edge) sur un **Raspberry Pi Compute Module 5** où Docker est déjà présent.  
Nous utiliserons **SQLite** comme base de données intégrée, sans serveur PostgreSQL supplémentaire, pour une installation plus légère.

---

## Prérequis

- Raspberry Pi 5 (ou Compute Module 5) avec **Docker** et **Docker Compose** installés.
- **Mosquitto** (MQTT) déjà opérationnel sous Docker (optionnel, pour les nœuds MQTT).
- **Git** installé (`sudo apt install git -y` si nécessaire).

---

## 1. Cloner le dépôt officiel de Z8RUN

```bash
git clone https://github.com/z8run/z8run.git
cd z8run
```

---

2. Créer le fichier d’environnement

```bash
cp .env.example .env
```

Générez une clé secrète pour les JWT et modifiez le fichier .env :

```bash
# Générer une clé forte
openssl rand -base64 32
```

Éditez .env (avec nano .env) et définissez au minimum :

```ini
Z8_JWT_SECRET=votre_clé_générée_ici
Z8_PUBLIC_PORT=7700
```

Note : Le port par défaut pour l’interface web est 80. Vous pouvez le modifier via Z8_PUBLIC_PORT.
Ici nous choisissons 7700 pour éviter les conflits.

---

3. Adapter le docker-compose.yml pour utiliser SQLite

Éditez le fichier docker-compose.yml (ou créez un fichier personnalisé). Voici un contenu minimal fonctionnel :

```yaml
version: '3.8'

services:
  z8run:
    image: ghcr.io/z8run/z8run-api:latest
    container_name: z8run
    restart: unless-stopped
    ports:
      - "${Z8_PUBLIC_PORT:-7700}:7700"
    environment:
      - Z8_JWT_SECRET=${Z8_JWT_SECRET}
      - Z8_DB_URL=sqlite:///app/data/z8run.db?mode=rwc
      - Z8_PUBLIC_PORT=${Z8_PUBLIC_PORT:-7700}
    volumes:
      - z8run_data:/app/data
    networks:
      - default

  z8run-web:
    image: ghcr.io/z8run/z8run-nginx:latest
    container_name: z8run-web
    restart: unless-stopped
    ports:
      - "${Z8_PUBLIC_PORT:-7700}:80"
    depends_on:
      - z8run
    networks:
      - default

volumes:
  z8run_data:

networks:
  default:
    # Vous pouvez connecter ce réseau à celui de Mosquitto si nécessaire
    # external: true
    # name: mosquitto_default
```

Important :

· La variable Z8_DB_URL=sqlite:///app/data/z8run.db?mode=rwc force l’utilisation de SQLite.
· Le volume z8run_data stocke la base de données SQLite (z8run.db) et les fichiers persistants.
· Le service z8run-web expose l’interface web sur le même port que l’API grâce à la variable ${Z8_PUBLIC_PORT}.

---

4. Démarrer Z8RUN

```bash
docker compose up -d
```

Les images compatibles ARM64 seront automatiquement téléchargées.

Vérifiez que les conteneurs tournent :

```bash
docker ps
```

Testez l’API :

```bash
curl http://localhost:7700/api/v1/health
```

Vous devriez obtenir une réponse {"status":"ok"}.

---

5. Accéder à l’interface web

Ouvrez un navigateur et rendez-vous sur :

```
http://<IP_DE_VOTRE_PI>:7700
```

Créez votre premier utilisateur (inscription) – les identifiants sont stockés dans SQLite.

---

6. Connecter Z8RUN à votre Mosquitto existant (optionnel)

Si votre conteneur Mosquitto s’appelle mosquitto et tourne sur le même réseau Docker, vous pouvez :

1. Créer un réseau Docker partagé :
   ```bash
   docker network create iot_network
   ```
2. Attacher Mosquitto à ce réseau :
   ```bash
   docker network connect iot_network mosquitto
   ```
3. Dans le docker-compose.yml de Z8RUN, ajoutez sous le service z8run :
   ```yaml
   networks:
     - default
     - iot_network
   ```
   et définissez le réseau externe en bas du fichier :
   ```yaml
   networks:
     default:
     iot_network:
       external: true
   ```
4. Redémarrez Z8RUN : docker compose up -d

Dès lors, dans les nœuds MQTT de Z8RUN, vous pourrez utiliser mosquitto:1883 comme adresse du broker.

---

7. Sauvegarde et mise à jour

· Sauvegarde : copiez le volume z8run_data (ex: docker run --rm -v z8run_data:/data -v $(pwd):/backup alpine tar czf /backup/z8run_backup.tar.gz /data).
· Mise à jour :
  ```bash
  docker compose pull
  docker compose up -d
  ```

---

Dépannage

Problème Solution
L’API ne répond pas Vérifiez les logs : docker logs z8run
SQLite – erreur readonly database Vérifiez les permissions du volume : docker exec -it z8run ls -la /app/data
Conflit de ports Modifiez Z8_PUBLIC_PORT dans .env et relancez
Mosquitto inaccessible Assurez-vous que les deux conteneurs partagent le même réseau Docker

---
