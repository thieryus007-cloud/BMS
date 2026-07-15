#!/usr/bin/env node
'use strict';

// Pont Remootio (WebSocket local chiffré) -> MQTT.
// Contourne les déconnexions fréquentes de l'app Homey Remootio : ce pont
// tourne en continu (idéalement sur le Pi5, à côté du broker), garde la
// session Remootio ouverte, et republie l'état + accepte les commandes
// via MQTT, que Homey consomme via un app MQTT générique (plus robuste).

const fs = require('fs');
const path = require('path');
const TOML = require('toml');
const mqtt = require('mqtt');
const RemootioDevice = require('remootio-api-client');

const CONFIG_PATH = process.env.REMOOTIO_MQTT_CONFIG || path.join(__dirname, 'config.toml');
const BRIDGE_AVAILABILITY_TOPIC = 'santuario/remootio/bridge/availability';

function loadConfig() {
  const raw = fs.readFileSync(CONFIG_PATH, 'utf8');
  return TOML.parse(raw);
}

function topicBase(name) {
  return `santuario/remootio/${name}`;
}

function setupDevice(dev, mqttClient) {
  const base = topicBase(dev.name);
  const stateTopic = `${base}/state`;
  const availTopic = `${base}/availability`;
  const setTopic = `${base}/set`;
  // Sortie 2 (impulse ctrl secondaire) : commande seule, l'API Remootio ne
  // remonte l'état (state/StateChange) que pour la sortie principale (Output 1).
  const secondarySetTopic = `${base}/secondary/set`;

  const pingIntervalMs = (dev.ping_interval_secs || 60) * 1000;
  const device = new RemootioDevice(dev.ip, dev.api_secret_key, dev.api_auth_key, pingIntervalMs);

  function publishState(state) {
    mqttClient.publish(
      stateTopic,
      JSON.stringify({ state, ts: Math.floor(Date.now() / 1000) }),
      { qos: 1, retain: true }
    );
    console.log(`[${dev.name}] état -> ${state}`);
  }

  device.on('connecting', () => console.log(`[${dev.name}] connexion en cours...`));

  device.on('connected', () => {
    console.log(`[${dev.name}] websocket connecté, authentification...`);
    device.authenticate();
  });

  device.on('authenticated', () => {
    console.log(`[${dev.name}] session authentifiée`);
    mqttClient.publish(availTopic, 'online', { qos: 1, retain: true });
    device.sendQuery();
  });

  device.on('disconnect', () => {
    console.warn(`[${dev.name}] déconnecté (reconnexion auto en cours)`);
    mqttClient.publish(availTopic, 'offline', { qos: 1, retain: true });
  });

  device.on('error', (err) => {
    console.error(`[${dev.name}] erreur:`, err && err.message ? err.message : err);
  });

  device.on('incomingmessage', (_frame, decrypted) => {
    if (!decrypted) return;
    if (decrypted.event && decrypted.event.type === 'StateChange') {
      publishState(decrypted.event.state);
    } else if (decrypted.response && decrypted.response.type === 'QUERY' && decrypted.response.state) {
      publishState(decrypted.response.state);
    }
  });

  mqttClient.subscribe([setTopic, secondarySetTopic], { qos: 1 });
  mqttClient.on('message', (topic, payload) => {
    if (topic !== setTopic && topic !== secondarySetTopic) return;
    const action = payload.toString().trim().toLowerCase();

    if (!device.isAuthenticated) {
      console.warn(`[${dev.name}] action '${action}' ignorée : session Remootio non authentifiée`);
      return;
    }

    if (topic === secondarySetTopic) {
      if (action === 'trigger') {
        device.sendTriggerSecondary();
      } else {
        console.warn(`[${dev.name}] action MQTT inconnue reçue sur ${secondarySetTopic}: '${action}' (seul 'trigger' est supporté sur la sortie secondaire)`);
      }
      return;
    }

    switch (action) {
      case 'open':
        device.sendOpen();
        break;
      case 'close':
        device.sendClose();
        break;
      case 'trigger':
        device.sendTrigger();
        break;
      case 'query':
        device.sendQuery();
        break;
      default:
        console.warn(`[${dev.name}] action MQTT inconnue reçue sur ${setTopic}: '${action}'`);
    }
  });

  device.connect(true);
}

function main() {
  const config = loadConfig();
  const mqttCfg = config.mqtt || {};
  const devices = config.devices || [];

  if (devices.length === 0) {
    console.error("Aucun appareil configuré ([[devices]] manquant dans config.toml)");
    process.exit(1);
  }

  const url = `mqtt://${mqttCfg.host || '127.0.0.1'}:${mqttCfg.port || 1883}`;
  const mqttClient = mqtt.connect(url, {
    username: mqttCfg.user,
    password: mqttCfg.password,
    will: { topic: BRIDGE_AVAILABILITY_TOPIC, payload: 'offline', qos: 1, retain: true },
  });

  mqttClient.on('connect', () => {
    console.log(`MQTT connecté (${url})`);
    mqttClient.publish(BRIDGE_AVAILABILITY_TOPIC, 'online', { qos: 1, retain: true });
  });

  mqttClient.on('error', (err) => console.error('MQTT erreur:', err.message));

  // setupDevice() enregistre un listener 'message' et un subscribe : à faire une
  // seule fois (mqtt.js met en file les publish/subscribe avant la connexion, et
  // republie l'abonnement automatiquement après une reconnexion).
  devices.forEach((dev) => setupDevice(dev, mqttClient));
}

main();
