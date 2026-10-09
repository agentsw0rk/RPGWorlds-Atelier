// Reine Logik der Oberfläche: Formular <-> Auftrag, Anzeigetexte, Galerie.
// Kein DOM, kein fetch — damit lässt sie sich mit `node --test web/logik.test.js` prüfen.
// Im Browser hängt sie als window.Logik, in Node kommt sie über require.
(function (wurzel) {
  'use strict';

  const STANDARD_PRESET = 'klein-9b';
  const MAX_BILDER = 64;
  const MIN_KANTE = 256;
  const MAX_KANTE = 2048;

  const artVon = (arten, kind) => (arten || []).find((a) => a.kind === kind) || {};
  const zahl = (wert) => (wert === '' || wert === null || wert === undefined ? undefined : Number(wert));

  // Das Formular als Auftrag für POST /jobs. Es werden nur Felder geschickt, die
  // vom Standard abweichen — so gelten die Vorgaben der API und ändern sich mit ihr.
  function auftragAusFormular(f, arten) {
    const art = artVon(arten, f.kind);
    const a = { kind: f.kind, prompt: String(f.prompt || '').trim() };

    a.seed = f.seedModus === 'fest' ? Number(f.seed) : -1;
    if (Number(f.varianten) > 1) a.seeds = Number(f.varianten);

    if (f.preset && f.preset !== STANDARD_PRESET) a.preset = f.preset;
    if (art.width && Number(f.width) !== art.width) a.width = Number(f.width);
    if (art.height && Number(f.height) !== art.height) a.height = Number(f.height);
    // Breite und Höhe nur gemeinsam, damit die API nie eine halbe Größe sieht.
    if (('width' in a) !== ('height' in a)) {
      a.width = Number(f.width);
      a.height = Number(f.height);
    }

    for (const feld of ['steps', 'cfg', 'guidance']) {
      const n = zahl(f[feld]);
      if (n !== undefined && !Number.isNaN(n)) a[feld] = n;
    }
    for (const feld of ['name', 'style']) {
      const t = String(f[feld] || '').trim();
      if (t) a[feld] = t;
    }
    if (f.kind === 'token' && String(f.pose || '').trim()) a.pose = String(f.pose).trim();
    if (f.kind === 'location' && f.large) a.large = true;

    if (f.freistellen === 'ja') a.freistellen = true;
    if (f.freistellen === 'nein') a.freistellen = false;

    if (f.stilref && f.stilref.id) {
      if (f.stilref.modus === 'inhalt') a.refs = [f.stilref.id];
      else a.style_ref = f.stilref.id;
      // Nur mit Referenz sinnvoll; leer = Standard des Servers.
      const px = zahl(f.refMaxPx);
      if (px !== undefined && !Number.isNaN(px)) a.ref_max_px = px;
    }
    return a;
  }

  // Prüft das Formular vor dem Senden. Gleiche Regeln wie die API, damit der
  // häufigste Fehler gar nicht erst einen Roundtrip braucht.
  function formularFehler(f) {
    if (!String(f.prompt || '').trim()) return 'Bitte einen Prompt eingeben.';
    for (const [name, wert] of [['Breite', f.width], ['Höhe', f.height]]) {
      const n = Number(wert);
      if (!Number.isFinite(n) || n < MIN_KANTE || n > MAX_KANTE) return `${name} muss zwischen ${MIN_KANTE} und ${MAX_KANTE} liegen.`;
      if (n % 16 !== 0) return `${name} muss durch 16 teilbar sein.`;
    }
    const v = Number(f.varianten);
    if (!Number.isInteger(v) || v < 1) return 'Mindestens eine Variante.';
    if (v > MAX_BILDER) return `Höchstens ${MAX_BILDER} Varianten pro Auftrag.`;
    return null;
  }

  // Umkehrung: ein bestehender Auftrag wird wieder zum Formular. Listenaufträge
  // haben keinen Prompt und lassen sich so nicht bearbeiten.
  function auftragInFormular(a, arten) {
    if (typeof a.prompt !== 'string') return null;
    const art = artVon(arten, a.kind);
    const stilref = a.style_ref ? { id: a.style_ref, modus: 'stil' }
      : a.refs && a.refs.length ? { id: a.refs[0], modus: 'inhalt' } : null;
    const feste = Array.isArray(a.seeds) ? a.seeds : null;
    return {
      kind: a.kind,
      prompt: a.prompt,
      varianten: feste ? feste.length : (typeof a.seeds === 'number' ? a.seeds : 1),
      seedModus: a.seed === undefined || a.seed === null || a.seed < 0 ? 'wuerfeln' : 'fest',
      seed: a.seed >= 0 ? a.seed : 42,
      preset: a.preset || STANDARD_PRESET,
      width: a.width || art.width || 1024,
      height: a.height || art.height || 1024,
      steps: a.steps === undefined ? '' : String(a.steps),
      cfg: a.cfg === undefined ? '' : String(a.cfg),
      guidance: a.guidance === undefined ? '' : String(a.guidance),
      refMaxPx: a.ref_max_px === undefined ? '' : String(a.ref_max_px),
      name: a.name || '',
      style: a.style || '',
      pose: a.pose || '',
      large: !!a.large,
      freistellen: a.freistellen === true ? 'ja' : a.freistellen === false ? 'nein' : 'auto',
      stilref,
    };
  }

  // "Mehr davon": derselbe Auftrag mit frischen Seeds. Das Original bleibt unberührt.
  function nochmal(auftrag, bild, anzahl) {
    const n = JSON.parse(JSON.stringify(auftrag));
    delete n.seeds;
    n.seed = -1;
    if (anzahl > 1) n.seeds = anzahl;
    if (n.liste) {
      // Bei Listen genau den Eintrag dieses Bildes wiederholen; ohne force
      // würde die API ihn als schon vorhanden überspringen.
      n.liste = { only: bild.eintrag, force: true };
    }
    return n;
  }

  const STATUS = {
    wartend: { text: 'wartet', klasse: 'wartet' },
    laeuft: { text: 'läuft', klasse: 'laeuft' },
    fertig: { text: 'fertig', klasse: 'fertig' },
    fehlgeschlagen: { text: 'fehlgeschlagen', klasse: 'fehler' },
    abgebrochen: { text: 'abgebrochen', klasse: 'aus' },
    vorhanden: { text: 'vorhanden', klasse: 'fertig' },
  };
  const statusInfo = (s) => STATUS[s] || { text: String(s), klasse: 'aus' };
  const istAktiv = (s) => s === 'wartend' || s === 'laeuft';

  function dauerText(s) {
    if (s === null || s === undefined) return '';
    if (s < 60) return `${s} s`;
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')} min`;
  }

  function zeitText(unix, jetzt) {
    const d = Math.max(0, jetzt - unix);
    if (d < 60) return 'gerade eben';
    if (d < 3600) return `vor ${Math.floor(d / 60)} min`;
    if (d < 86400) return `vor ${Math.floor(d / 3600)} h`;
    const tage = Math.floor(d / 86400);
    return `vor ${tage} ${tage === 1 ? 'Tag' : 'Tagen'}`;
  }

  const fortschritt = (j) => (j.bilder_gesamt ? Math.round((100 * j.bilder_fertig) / j.bilder_gesamt) : 0);

  // Alle fertigen Bilder, neueste Jobs zuerst; innerhalb eines Jobs bleibt die Reihenfolge.
  function galerie(jobs, art) {
    return [...jobs]
      .filter((j) => !art || art === 'alle' || j.kind === art)
      .sort((a, b) => b.erstellt - a.erstellt)
      .flatMap((job) => job.bilder.filter((b) => b.status === 'fertig' && b.url).map((bild) => ({ job, bild })));
  }

  function vorschauKopf(plan) {
    const n = plan.bilder.length;
    return `${n} ${n === 1 ? 'Bild' : 'Bilder'} · ${plan.groesse[0]}×${plan.groesse[1]} · ${plan.steps} Steps · ${plan.preset}`;
  }

  const downloadName = (bild) => bild.datei || `${bild.stamm}.png`;

  function fehlerText(antwort, status) {
    if (antwort && antwort.fehler) return antwort.fehler;
    if (status === 401) return 'Token fehlt oder stimmt nicht.';
    return `Fehler ${status}`;
  }

  // ---- Metriken: Balken aus einem Messpunkt (`zusammenfassung.aktuell` der API)

  const gb = (mb) => (mb / 1024).toFixed(1).replace('.', ',');
  const anteil = (teil, gesamt) => (teil <= 0 ? 0 : gesamt <= 0 ? 100 : Math.min(100, Math.round((100 * teil) / gesamt)));
  const stufeFuer = (prozent, warn, kritisch) => (prozent >= kritisch ? 'kritisch' : prozent >= warn ? 'warn' : 'ok');

  // Höchstwerte eines Laufs als Messpunkt — für beendete Jobs, die keinen "aktuellen" Stand mehr haben.
  function spitzenPunkt(z) {
    if (!z.aktuell) return null;
    return {
      ...z.aktuell,
      rss_mb: z.spitze_rss_mb, frei_mb: z.min_frei_mb, swap_mb: z.max_swap_mb, cpu_pct: z.spitze_cpu_pct,
      swap_ein_mb_s: z.spitze_swap_ein_mb_s || 0, swap_aus_mb_s: z.spitze_swap_aus_mb_s || 0,
    };
  }

  const PHASEN = {
    referenz: 'Referenz vorbereiten', erzeugen: 'Bild erzeugen', freistellen: 'Freistellen', bild_fertig: 'Bild fertig',
  };
  const phasenText = (name) => (name ? PHASEN[name] || name : '');

  function metrikBalken(m) {
    const belegt = m.gesamt_mb - m.frei_mb;
    const rss = anteil(m.rss_mb, m.gesamt_mb);
    const system = anteil(belegt, m.gesamt_mb);
    const swap = anteil(m.swap_mb, m.swap_gesamt_mb);
    const ein = m.swap_ein_mb_s || 0;
    const aus = m.swap_aus_mb_s || 0;
    const rate = Math.max(ein, aus);
    const auslagerung = anteil(rate, 200);
    const komma = (x) => x.toFixed(1).replace('.', ',');
    return [
      { schluessel: 'rss', label: 'Prozess', prozent: rss, text: `${gb(m.rss_mb)} GB`, stufe: stufeFuer(rss, 70, 85) },
      { schluessel: 'system', label: 'System belegt', prozent: system, text: `${gb(belegt)} von ${gb(m.gesamt_mb)} GB`, stufe: stufeFuer(system, 75, 90) },
      { schluessel: 'swap', label: 'Swap', prozent: swap, text: `${gb(m.swap_mb)} von ${gb(m.swap_gesamt_mb)} GB`, stufe: m.swap_mb === 0 ? 'ok' : stufeFuer(swap, 0, 50) },
      {
        schluessel: 'auslagerung', label: 'Auslagerung', prozent: auslagerung,
        text: `ein ${komma(ein)} · aus ${komma(aus)} MB/s`, stufe: rate === 0 ? 'ok' : stufeFuer(rate, 0.001, 20),
      },
    ];
  }

  const esc = (t) => String(t === null || t === undefined ? '' : t)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

  const api = {
    STANDARD_PRESET, MAX_BILDER,
    auftragAusFormular, formularFehler, auftragInFormular, nochmal,
    statusInfo, istAktiv, dauerText, zeitText, fortschritt, galerie,
    vorschauKopf, downloadName, fehlerText, metrikBalken, spitzenPunkt, phasenText, esc,
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else wurzel.Logik = api;
})(typeof window !== 'undefined' ? window : globalThis);
