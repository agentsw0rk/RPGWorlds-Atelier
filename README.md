<div align="center">

# 🎲 RPGWorlds Atelier

**Beschreib eine Figur oder einen Ort in einem Satz – das Atelier malt dir das Bild dazu.**
Für deine Rollenspielrunde, dein Spielbrett, dein Heft voller Ideen.

![Läuft lokal](https://img.shields.io/badge/läuft-komplett_lokal-4c9a2a)
![Apple Silicon](https://img.shields.io/badge/Mac-Apple_Silicon-555)
![Rust](https://img.shields.io/badge/gebaut_mit-Rust-b7410e)
![Kein Python](https://img.shields.io/badge/Python-nicht_nötig-3776ab)

</div>

---

## ✨ Worum geht's?

Du spielst **D&D** oder ein anderes Fantasy-Rollenspiel und hättest gern Bilder für deine Figuren,
Gegner und Schauplätze? Dann ist das hier dein eigenes kleines Zeichenstudio:

> *„Eine zwölfjährige Elfenwaise mit abgewetztem Mantel, trägt ein viel zu großes Schwert“*
> → ein fertiges Bild, auf Wunsch **mit freigestelltem Hintergrund**, damit du die Figur
> direkt auf eine Karte oder ein Spielbrett legen kannst.

Alles läuft **auf deinem eigenen Rechner**. Kein Konto, keine Cloud, keine Abo-Gebühr. Deine
Ideen und Bilder bleiben bei dir. Nur die KI-Modelle werden **einmalig** heruntergeladen.

Unter der Haube arbeitet [FLUX.2 klein](https://huggingface.co/black-forest-labs/FLUX.2-klein-4B)
(ein Bildmodell) – gesteuert von einem Rust-Programm, ganz ohne Python.

## 🖼️ Was kann es?

| Art | Was du bekommst | Ideal für |
|---|---|---|
| 🪙 **Token** | Figur von Kopf bis Fuß, **Hintergrund transparent** | Spielbrett, virtuelle Tabletops |
| 🧑‍🎤 **Porträt** | Kopf und Schultern, füllt das Bild | Charakterbögen, NPC-Karten |
| 🧍 **Ganzkörper** | Stehende Figur mitten in einer Szene | Illustrationen, Titelbilder |
| 🏰 **Ort** | Komplette Landschaft oder Stadt | Kartenbilder, Szenenvorlesen |

Dazu kommt:

- 🎨 **Referenzbild**: Lade ein Bild hoch und das Atelier übernimmt entweder nur den *Stil*
  (Malweise, Farben, Licht) **oder** die *Szene* (Ort, Perspektive) und setzt deine Figur hinein.
- 🎰 **Varianten**: Hände und Waffen sind Würfelglück. Lass dir 4 Bilder auf einmal würfeln und
  nimm das beste.
- 📚 **Fertige Listen**: über **450 Charaktere** (mit Porträt-Variante), **77 Orte** aus bekannten
  Fantasy-Welten und eine kleine Kinderheim-Serie. Ein Klick – das Atelier arbeitet alles ab.
- ✂️ **Automatisch freistellen**: Der Hintergrund wird für dich entfernt, ganz ohne zusätzliches Programm.
- 📊 **Live-Anzeige**: Du siehst, wie viel Speicher und Rechenpower dein Bild gerade braucht.

## 🚀 Los geht's

### Das brauchst du

- Einen **Mac mit Apple-Chip** (M1 oder neuer). Getestet mit **32 GB** Arbeitsspeicher.
- Etwa **10–20 GB freien Platz** für die Modelle.
- Ein bisschen Geduld beim ersten Start (alles wird einmal gebaut und geladen).
- Drei Werkzeuge, falls noch nicht da:

```sh
xcode-select --install                                         # Compiler-Werkzeuge von Apple
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh # Rust
brew install cmake                                             # Bauwerkzeug
```

### Starten

```sh
./start.sh
```

Dann im Browser **http://127.0.0.1:8080** öffnen. Fertig. 🎉

Beim allerersten Start wird das Programm einmal gebaut (das dauert ein paar Minuten), und beim
allerersten Bild lädt das Atelier die Modelle von selbst herunter (mehrere GB; ein
abgebrochener Download wird einfach fortgesetzt).

> 💡 **Erst mal nur gucken?** Mit `./start.sh --demo` siehst du die ganze Oberfläche mit bunten
> Platzhalterbildern, ohne irgendetwas herunterzuladen und ohne Rechenzeit.

### Dein erstes Bild

1. Oben auf **Erstellen** gehen und eine **Art** wählen, zum Beispiel *Token*.
2. Die Figur beschreiben, am besten **auf Englisch**, weil das Modell damit am besten klarkommt:
   `a grumpy dwarf blacksmith with a braided red beard, leather apron, heavy hammer`
3. Auf **Vorschau** klicken, um den fertigen Auftrag zu sehen (kostet keine Rechenzeit), dann auf **Generieren**.
4. Im Reiter **Jobs** zuschauen, im Reiter **Galerie** bewundern und herunterladen. 🥳

## ⏱️ Wie lange dauert das?

Ehrlich gesagt: **eine Weile.** Bei uns (Mac mit 32 GB, Bild 768 × 1536, mit Referenzbild):

| Modell | ungefähr |
|---|---|
| `klein-4b` (kleiner, schneller) | **9–12 Minuten** pro Bild |
| `klein-9b` (Standard, schöner) | **etwa 17 Minuten** pro Bild |

Die Zeit geht fast komplett ins „Malen“ selbst. Mit diesen Tricks wird's flotter:

- 🐇 **Kleineres Modell** wählen (`klein-4b`).
- 🖼️ **Referenzbild verkleinern**: Unter *Weitere Einstellungen → Referenzgröße* z. B. 512 px
  wählen. Das Modell hat dann weniger zu tun.
- 📐 **Kleineres Bild** erzeugen.
- 🌙 **Über Nacht laufen lassen**: Mit den Listen oder mehreren Varianten stapelt das Atelier die
  Aufträge und arbeitet sie nacheinander ab.

Die Zahlen stammen aus echten Läufen mit eingeschalteter Messung; auf deinem Gerät kann es
anders aussehen.

## 🧙 Tipps für bessere Bilder

- **Sag, was du willst – nicht, was du nicht willst.** „*not chibi*“ macht das Modell erst recht
  zu Chibi. Schreib lieber, wie die Figur aussieht: „*seven and a half heads tall, mature face*“.
- **Mit Szenen-Referenz nur die Figur beschreiben.** Schreibst du zusätzlich den Hintergrund
  hinein, kämpft dein Text gegen dein Bild.
- **Würfeln lohnt sich.** Füße, Hände und gedrehte Körper gelingen nicht immer. Mehr Varianten =
  mehr Chancen.
- **Kamera in einfachen Worten:** „*high angle view looking down on the figure*“ funktioniert,
  Fachbegriffe wie „30-degree elevated three-quarter view“ oft nicht.
- **Ein Referenzbild hilft mehr als jede Beschreibung**, wenn du einen bestimmten Blickwinkel willst.

> ⚠️ **Fair bleiben:** Lade nur Bilder als Referenz hoch, die du selbst gemacht hast oder
> verwenden darfst. Und prüfe die Lizenzen der Modelle, bevor du Bilder öffentlich oder
> kommerziell nutzt (siehe unten).

## 🧠 Welches Modell nehme ich?

Du wählst unter *Weitere Einstellungen → Modell*:

| Name | In einem Satz | Zugang |
|---|---|---|
| `klein-4b` | Das kleine, flinke Modell. Gut zum Ausprobieren. | offen |
| `klein-9b` | **Standard.** Mehr Details, braucht mehr Speicher und Zeit. | offen |
| `klein-base-9b` | Die nicht „destillierte“ Basisversion: 20 statt 4 Schritte, also deutlich langsamer. Für Tüftler. | braucht ein [Hugging-Face](https://huggingface.co)-Konto und Token |

Das Token gibst du als `HF_TOKEN` an oder legst es in eine Datei `hf-token.env` im Projektordner
(die wird nie hochgeladen, dafür sorgt die `.gitignore`).

## 🛟 Hilfe, mein Mac hängt!

Das passiert, wenn das Modell mehr Arbeitsspeicher will, als frei ist, und der Mac anfängt,
Daten auf die Festplatte auszulagern. Probier:

```sh
FLASH_ATTENTION=1 VAE_TILING=1 ./start.sh
```

Das spart Speicher. Bei uns hat das Hängen damit aufgehört (ein Garant ist das nicht). Außerdem hilft:
andere große Programme schließen, `klein-4b` nehmen oder die Referenz verkleinern.

**Du willst wissen, wo die Zeit und der Speicher hingehen?** Das Atelier misst bei jedem Auftrag
mit (Datei `api-daten/jobs/<id>/metrics.jsonl`). In der Jobliste siehst du Live-Balken. Danach:

```sh
./flux2-rs/target/release/metrics-report api-daten/jobs/<id>/metrics.jsonl
```

zeigt dir Phase für Phase, wie viel Speicher gebraucht wurde und wann der Mac auslagern musste.

## 🔌 Für Bastler: die Schnittstelle (API)

Hinter der Oberfläche steckt eine kleine HTTP-API. Du kannst Bilder also auch aus eigenen
Programmen bestellen:

```sh
curl -X POST localhost:8080/jobs \
  -d '{"kind":"token","prompt":"a grumpy dwarf blacksmith, braided red beard","seeds":4}'
```

Alle Routen und Optionen stehen in der [technischen Dokumentation](flux2-rs/README.md#http-api-flux2-api).

## 🗂️ Was liegt wo?

```
RPGWorlds Atelier
├── start.sh               ← startet das Atelier
├── flux2-rs/              ← der ganze Code (Rust) + Doku
│   ├── src/               ← Programm: Bilder erzeugen, freistellen, API
│   ├── web/               ← die Weboberfläche
│   ├── scripts/           ← Shell-Skripte für die Kommandozeile
│   ├── charaktere.txt     ← 450 Figuren-Ideen
│   ├── orte.txt           ← 77 Schauplätze
│   └── README.md          ← technische Doku (Details, Optionen, Stolpersteine)
├── models/                ← die KI-Modelle (mehrere GB, nicht im Repo)
└── tokens/ …              ← deine fertigen Bilder (nicht im Repo)
```
---
<div align="center">

Viel Spaß beim Würfeln und Malen! 🎲🖌️🐉

</div>
