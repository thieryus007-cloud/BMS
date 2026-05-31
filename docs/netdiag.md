sudo bash scripts/netdiag.sh
════════════════════════════════════════════════════════════
 netdiag — capture du 2026-05-31 11:51:40  (fenêtre 10s)
════════════════════════════════════════════════════════════

▶ TX total (toutes ifaces non-lo) : 11010 B/s  (~10 Ko/s, 0.09 Mbit/s)

▶ Débit TX par interface :
  eth0              0 B/s
  wlan0         11206 B/s

▶ Top connexions par octets ENVOYÉS sur la fenêtre :
         B/s  local                   pair                    process
        9516  192.168.1.120:1883      users:(("mosquitto",pid=1194,fd=11))  mosquitto  [mosquitto]
        8939  127.0.0.1:1883          users:(("daly-bms-server",pid=1217,fd=14))  daly-bms-server  [mosquitto]
        2978  127.0.0.1:45060         users:(("mosquitto",pid=1194,fd=15))  mosquitto
        2640  127.0.0.1:1883          users:(("energy-manager",pid=1310,fd=9))  energy-manager  [mosquitto]
        1474  127.0.0.1:45080         users:(("mosquitto",pid=1194,fd=16))  mosquitto
         405  192.168.1.141:54878     users:(("daly-bms-server",pid=1217,fd=26))  daly-bms-server
         314  192.168.1.141:8080      users:(("energy-manager",pid=1310,fd=13))  energy-manager  [daly-bms (API/WS)]
           3  192.168.1.116:12451     users:(("sshd-session",pid=1253,fd=7),("sshd-session",pid=1244,fd=7))  sshd-session

▶ Connexions établies par port local (count) :
     6  :1883   mosquitto
     4  :8080   daly-bms (API/WS)
     1  :54890  
     1  :54878  
     1  :45080  
     1  :45060  
     1  :45032  
     1  :45030  
     1  :45014  
     1  :38842  

▶ Top process CPU :
      PID COMMAND         %CPU %MEM
     1217 daly-bms-server  1.2  1.9
     1193 grafana          0.4  6.8
      606 kworker/u17:2-b  0.3  0.0
     1194 mosquitto        0.2  0.3
     1310 energy-manager   0.1  0.2
      299 systemd-journal  0.0  0.5
     4359 kworker/0:2-eve  0.0  0.0

▶ MQTT broker $SYS (1 min) :
  load/bytes/sent/1min       874924.87
  load/messages/sent/1min    4551.56
  clients/connected          6
