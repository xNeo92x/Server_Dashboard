# Server Dashboard

## Vorschau

![Server Dashboard Vorschau](docs/server-dashboard-preview.jpg)

Native Linux-Anwendung zur übersichtlichen Darstellung der Gesundheit mehrerer
Server aus **N-able N-sight RMM**. Das Projekt ist für einen dauerhaft an einem
Fernseher betriebenen Raspberry Pi bzw. Debian-Rechner gedacht.

Die Anwendung ist vollständig in **Rust + GTK4** umgesetzt. Es gibt keinen
Browser, keine WebView, keinen eingebetteten Webserver und kein JavaScript.

## Design

Das Dashboard übernimmt bewusst wesentliche Gestaltungsprinzipien von
[Overlook](https://github.com/bnlqn/overlook):

- sehr dunkle, ruhige Oberfläche mit eingefasstem Hauptpanel
- kompakte linke Navigation
- dünne Hairline-Rahmen statt großer Material-Schatten
- klare Grün/Gelb/Rot/Grau-Zustände
- Status-Headline und Problem-Inbox im oberen Bereich
- kompakte Kennzahlen und tabellarische Serverübersicht
- kleine Typografie und hohe Informationsdichte für einen TV-Wallboard-Betrieb

Die Oberfläche ist keine eingebettete Overlook-Web-App, sondern eine
GTK4-Neuimplementierung. **Die Apps-Kategorie von Overlook wurde absichtlich
nicht übernommen.** Es gibt ausschließlich Server- und Monitoring-Bereiche.

Siehe [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Funktionen

- native GTK4-Anwendung für Debian / Raspberry Pi OS
- N-able N-sight RMM als Datenquelle
- Gesamtzustand: Gesund / Warnung / Kritisch / Unbekannt
- Problem-Inbox für Server, die Aufmerksamkeit benötigen
- Kennzahlen für Server, gesunde Server, Problemserver und Monitoring-Checks
- kompakte Server-Tabelle mit CPU, RAM, Datenträger, Check-Anzahl und letzter Rückmeldung
- Detailfenster mit den von N-sight gelieferten Monitoring-Checks
- automatische Aktualisierung alle 120 Sekunden
- automatische Seitenschaltung alle 22 Sekunden bei mehr als 10 Servern
- F11 schaltet Vollbild um, Escape verlässt den Vollbildmodus
- Offline-Demomodus mit `--demo`
- optionale Einschränkung auf Site-IDs oder Device-IDs
- kein App-Launcher und keine Apps-Kategorie

## Sicherheitsmodell für den API-Token

Der N-sight API-Token wird **bei jedem Programmstart** über ein Passwortfeld
abgefragt. Die Anwendung besitzt absichtlich keine Option, ihn über ein
Kommandozeilenargument, eine Umgebungsvariable oder eine Konfigurationsdatei zu
übergeben.

Nach der Übernahme aus dem Passwortfeld wird der Token mit
**XChaCha20-Poly1305** und einem zufälligen 256-Bit-Prozessschlüssel
verschlüsselt. Der Schlüssel liegt in einem mittels `mlock()` gesperrten
Speicherbereich. Klartextpuffer, die für einen API-Aufruf benötigt werden,
werden ebenfalls mit `mlock()` vom Swap ferngehalten und anschließend mit
`zeroize` überschrieben.

Zusätzlich setzt die App unter Linux:

- `RLIMIT_CORE=0`
- `PR_SET_DUMPABLE=0`
- keine HTTP-Redirects
- ausschließlich HTTPS für den N-sight-Endpunkt
- keine automatisch aus der Umgebung übernommenen HTTP-Proxys
- keine Ausgabe roher Request-URLs in Benutzerfehlern

### Wichtige technische Grenze

N-sight erwartet den API-Schlüssel als `apikey` im HTTPS-Querystring. Für den
Aufbau eines Requests muss die App ihn daher kurz entschlüsseln. Während dieser
Zeit existiert der Schlüssel unvermeidbar als Klartext im Prozess und im
Request-URL-Objekt der HTTP-Bibliothek. Zwischen den Requests hält die App den
Token nur verschlüsselt.

Auch das GTK-Passwortfeld wird nach der Übernahme sofort geleert. GTK/glib und
der Systemallocator können jedoch nicht garantieren, dass jede frühere interne
Kopie eines eingegebenen Textes physisch überschrieben wurde. Root bzw. ein
vollständig kompromittierter Prozess kann Arbeitsspeicher grundsätzlich
untersuchen.

## Voraussetzungen

Empfohlen wird ein **64-Bit Debian / Raspberry Pi OS**.

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libgtk-4-dev ca-certificates curl
```

Benötigt wird Rust 1.77 oder neuer.

## Bauen

```bash
git clone https://github.com/xNeo92x/Server_Dashboard.git
cd Server_Dashboard
cargo test
cargo build --release
```

Die Binärdatei befindet sich anschließend unter
`target/release/server-dashboard`.

### Demo starten

```bash
./target/release/server-dashboard --demo
```

### Mit N-sight starten

```bash
./target/release/server-dashboard --fullscreen
```

Die App fragt den API-Token anschließend grafisch ab. Nicht geheime Optionen
dürfen vorgegeben werden:

```bash
./target/release/server-dashboard \
  --api-host https://wwwgermany1.systemmonitor.eu.com \
  --device-ids 12345,67890 \
  --fullscreen
```

Oder nach Sites:

```bash
./target/release/server-dashboard --site-ids 1111,2222 --fullscreen
```

Der Token kann bewusst **nicht** als CLI-Parameter angegeben werden.

## Installation unter Debian

```bash
chmod +x install.sh
./install.sh
```

Installiert werden:

```text
/opt/server-dashboard/server-dashboard
/usr/share/applications/server-dashboard.desktop
```

## Debian-Paket erstellen

```bash
chmod +x packaging/build-deb.sh
./packaging/build-deb.sh
```

Das Paket landet anschließend in `target/`.

## Autostart auf dem Raspberry Pi

```bash
mkdir -p ~/.config/autostart
cp /usr/share/applications/server-dashboard.desktop ~/.config/autostart/
```

Bei Bedarf kann `Exec=` um nicht geheime Parameter erweitert werden:

```ini
Exec=/opt/server-dashboard/server-dashboard --device-ids 12345,67890 --fullscreen
```

Nach jedem Neustart erscheint absichtlich wieder die Token-Abfrage.

## N-sight-Daten

Der Client verwendet ausschließlich lesende N-sight-Endpunkte:

- `list_servers`
- `list_device_monitoring_details`

Die XML-Antworten werden begrenzt verarbeitet, DTD/Entity-Deklarationen werden
abgelehnt, Redirects werden nicht verfolgt und die Request-Frequenz wird
begrenzt.

CPU-, RAM- und Datenträgerwerte sind nicht in jedem N-sight-Mandanten als
universelle Live-Telemetrie vorhanden. Server Dashboard extrahiert solche Werte
nur dann, wenn vorhandene Monitoring-Checks einen eindeutig beschrifteten
Prozentwert liefern. Ansonsten wird `—` angezeigt.

## Projektstruktur

```text
src/main.rs        Anwendung und sichere CLI-Optionen
src/security.rs    XChaCha20-Poly1305, mlock, zeroize, Prozess-Härtung
src/api.rs         N-sight HTTPS/XML-Client
src/model.rs       Server-, Check- und Snapshot-Modelle
src/ui.rs          native GTK4-Oberfläche im Overlook-inspirierten Design
src/demo.rs        lokale Demo-Daten
assets/style.css   GTK4-Theme
packaging/         Desktop-Datei und .deb-Build
.github/workflows  CI-Build unter Ubuntu/GTK4
```

## CI

GitHub Actions führt Tests und einen Release-Build auf Ubuntu mit den
GTK4-Entwicklungspaketen aus.

## Datenschutz

Server Dashboard enthält keine Telemetrie und keinen Update-Check. Es schreibt
weder API-Token noch Serverhistorie in eine lokale Datenbank. Der aktuelle
Snapshot existiert nur während der laufenden Sitzung im Arbeitsspeicher.

## Lizenz

Server Dashboard steht unter der MIT-Lizenz. Das von Overlook inspirierte
Design und dessen MIT-Hinweis sind in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) dokumentiert.
