Voici la procédure complète pour installer Windmill sur Windows via Docker.

Prérequis indispensables

Avant de commencer, assurez-vous d'avoir installé Docker Desktop for Windows. Vous pouvez le télécharger et l'installer en suivant la procédure sur le site officiel de Docker. C'est l'outil qui va nous permettre de faire tourner Windmill dans des conteneurs.

1. Récupérer la configuration (Docker Compose)

Windmill fournit un fichier docker-compose.yml prêt à l'emploi qui définit l'ensemble des services nécessaires (le serveur Windmill, ses workers, la base de données PostgreSQL, etc.). Ouvrez un terminal (PowerShell ou Invite de commandes) et créez un répertoire dédié pour éviter de mélanger les fichiers.

```bash
mkdir windmill
cd windmill
```

Téléchargez ensuite les deux fichiers nécessaires depuis le dépôt officiel de Windmill sur GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/windmill-labs/windmill/main/docker-compose.yml -o docker-compose.yml .

curl -fsSL https://raw.githubusercontent.com/windmill-labs/windmill/main/.env -o .env. .


```

2. Configuration personnalisée

C'est une étape importante pour la sécurité de votre instance. Ouvrez le fichier .env que vous venez de télécharger avec un éditeur de texte (comme le Bloc-Notes). Vous allez modifier au minimum les variables suivantes:

· WINDMILL_SUPERADMIN_EMAIL: Remplacez l'adresse par défaut par votre propre email.
· WINDMILL_SUPERADMIN_PASSWORD: Choisissez un mot de passe très fort, complexe et unique. C'est le mot de passe administrateur principal.
· SECRET: Vous pouvez aussi changer cette valeur, qui sert de clé de chiffrement. Utilisez une longue chaîne de caractères aléatoires.
· (Optionnel) WM_BASE_URL: Si vous comptez accéder à Windmill depuis un autre appareil sur votre réseau, remplacez http://localhost:8000 par l'adresse IP de votre machine. Par exemple, si l'IP de votre PC est 192.168.1.10 et que vous voulez utiliser le port 8000, indiquez http://192.168.1.10:8000.

3. Démarrer Windmill

Une fois la configuration faite, le lancement est très simple. Dans votre terminal, toujours à la racine du répertoire windmill, exécutez la commande suivante:

```bash
docker-compose up -d
```

L'option -d fait tourner le tout en arrière-plan (mode "détaché"). Le processus peut prendre quelques minutes le temps de télécharger toutes les images nécessaires (serveur, base de données, etc.).

4. Accéder à l'interface

Lancez votre navigateur web et rendez-vous à l'adresse suivante: http://localhost:8000.

Vous devriez voir apparaître la page de connexion de Windmill. C'est ici que vous utiliserez l'email et le mot de passe administrateur que vous avez configurés dans le fichier .env.

5. Gérer l'installation

· Consulter les logs: Pour voir ce qui se passe en direct (utile en cas d'erreur), utilisez:
  ```bash
  docker-compose logs -f
  ```
· Arrêter Windmill: La commande suivante arrête tous les conteneurs sans les supprimer:
  ```bash
  docker-compose down
  ```
· Mettre à jour: Pour passer à une nouvelle version, arrêtez les conteneurs (down), puis relancez avec docker-compose pull pour récupérer les dernières images, suivi de docker-compose up -d.

---

Un petit conseil pour finir : Si vous prévoyez une utilisation régulière, n'oubliez pas de sécuriser votre accès avec HTTPS (par exemple en utilisant un proxy comme Caddy ou Traefik) et de penser à la sauvegarde régulière des données de votre base de données PostgreSQL.

Si vous rencontrez le moindre souci, n'hésitez pas à me le dire！
