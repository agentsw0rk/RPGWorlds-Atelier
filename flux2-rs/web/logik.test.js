// Tests der reinen Oberflächenlogik: node --test web/logik.test.js
const test = require('node:test');
const assert = require('node:assert/strict');
const L = require('./logik.js');

const ARTEN = [
  { kind: 'fullbody', width: 768, height: 1536, steps: 8, freistellen: false },
  { kind: 'token', width: 1024, height: 1024, steps: 8, freistellen: true },
  { kind: 'location', width: 1024, height: 512, steps: 8, freistellen: false },
];
const form = (extra = {}) => ({
  kind: 'fullbody', prompt: 'dwarf smith', varianten: 1, seedModus: 'wuerfeln', seed: 42,
  preset: 'klein-9b', width: 768, height: 1536, steps: '', cfg: '', guidance: '',
  name: '', style: '', pose: '', large: false, freistellen: 'auto', stilref: null, ...extra,
});

test('einfaches Formular schickt nur kind, prompt und den Würfel-Seed', () => {
  const a = L.auftragAusFormular(form(), ARTEN);
  assert.deepEqual(a, { kind: 'fullbody', prompt: 'dwarf smith', seed: -1 });
});

test('der Prompt wird beschnitten', () => {
  assert.equal(L.auftragAusFormular(form({ prompt: '  dwarf  ' }), ARTEN).prompt, 'dwarf');
});

test('mehrere Varianten werden als seeds-Anzahl geschickt', () => {
  const a = L.auftragAusFormular(form({ varianten: 4 }), ARTEN);
  assert.equal(a.seeds, 4);
  assert.equal(a.seed, -1);
});

test('fester Seed wird übernommen, bei einer Variante ohne seeds-Feld', () => {
  const a = L.auftragAusFormular(form({ seedModus: 'fest', seed: 123 }), ARTEN);
  assert.equal(a.seed, 123);
  assert.equal('seeds' in a, false);
});

test('Größe nur senden, wenn sie vom Standard der Art abweicht', () => {
  assert.equal('width' in L.auftragAusFormular(form(), ARTEN), false);
  const a = L.auftragAusFormular(form({ width: 1024, height: 2048 }), ARTEN);
  assert.deepEqual([a.width, a.height], [1024, 2048]);
  const t = L.auftragAusFormular(form({ kind: 'token', width: 1024, height: 1024 }), ARTEN);
  assert.equal('width' in t, false);
});

test('Preset nur senden, wenn es nicht der Default ist', () => {
  assert.equal('preset' in L.auftragAusFormular(form(), ARTEN), false);
  assert.equal(L.auftragAusFormular(form({ preset: 'klein-4b' }), ARTEN).preset, 'klein-4b');
});

test('leere Zusatzfelder werden weggelassen, gefüllte als Zahl bzw. Text geschickt', () => {
  const leer = L.auftragAusFormular(form(), ARTEN);
  for (const k of ['steps', 'cfg', 'guidance', 'name', 'style', 'pose']) assert.equal(k in leer, false, k);
  const a = L.auftragAusFormular(form({ steps: '12', cfg: '1.5', guidance: '3', name: 'mein-bild' }), ARTEN);
  assert.deepEqual([a.steps, a.cfg, a.guidance, a.name], [12, 1.5, 3, 'mein-bild']);
});

test('pose gilt nur für Tokens, large nur für Orte', () => {
  assert.equal('pose' in L.auftragAusFormular(form({ pose: 'arms crossed' }), ARTEN), false);
  assert.equal(L.auftragAusFormular(form({ kind: 'token', width: 1024, height: 1024, pose: 'arms crossed' }), ARTEN).pose, 'arms crossed');
  assert.equal(L.auftragAusFormular(form({ kind: 'location', width: 1024, height: 512, large: true }), ARTEN).large, true);
  assert.equal('large' in L.auftragAusFormular(form({ large: true }), ARTEN), false);
});

test('Freistellen: auto lässt das Feld weg, ja/nein setzt es', () => {
  assert.equal('freistellen' in L.auftragAusFormular(form(), ARTEN), false);
  assert.equal(L.auftragAusFormular(form({ freistellen: 'ja' }), ARTEN).freistellen, true);
  assert.equal(L.auftragAusFormular(form({ freistellen: 'nein' }), ARTEN).freistellen, false);
});

test('Stilreferenz geht als style_ref, Inhaltsreferenz als refs', () => {
  const stil = L.auftragAusFormular(form({ stilref: { id: 'abc', modus: 'stil' } }), ARTEN);
  assert.equal(stil.style_ref, 'abc');
  assert.equal('refs' in stil, false);
  const inhalt = L.auftragAusFormular(form({ stilref: { id: 'abc', modus: 'inhalt' } }), ARTEN);
  assert.deepEqual(inhalt.refs, ['abc']);
  assert.equal('style_ref' in inhalt, false);
});

test('leerer Prompt ist ein Fehler, den die Oberfläche vor dem Senden meldet', () => {
  assert.equal(L.formularFehler(form({ prompt: '  ' })), 'Bitte einen Prompt eingeben.');
  assert.equal(L.formularFehler(form()), null);
  assert.match(L.formularFehler(form({ width: 300 })), /durch 16/);
  assert.match(L.formularFehler(form({ width: 0 })), /Breite/);
  assert.match(L.formularFehler(form({ height: '' })), /Höhe/);
  assert.match(L.formularFehler(form({ width: 4096 })), /256/);
  assert.match(L.formularFehler(form({ varianten: 0 })), /Variante/);
  assert.match(L.formularFehler(form({ varianten: 100 })), /64/);
});

test('Auftrag lässt sich ins Formular zurückübernehmen', () => {
  const a = { kind: 'token', prompt: 'elf', seeds: 3, seed: -1, preset: 'klein-4b', width: 1024, height: 1536, steps: 10, style_ref: 'abc', freistellen: false };
  const f = L.auftragInFormular(a, ARTEN);
  assert.equal(f.kind, 'token');
  assert.equal(f.prompt, 'elf');
  assert.equal(f.varianten, 3);
  assert.equal(f.seedModus, 'wuerfeln');
  assert.equal(f.preset, 'klein-4b');
  assert.deepEqual([f.width, f.height, f.steps], [1024, 1536, '10']);
  assert.deepEqual(f.stilref, { id: 'abc', modus: 'stil' });
  assert.equal(f.freistellen, 'nein');
});

test('Rückübernahme setzt Standardwerte der Art, wo der Auftrag nichts sagte', () => {
  const f = L.auftragInFormular({ kind: 'fullbody', prompt: 'x', seed: 7 }, ARTEN);
  assert.deepEqual([f.width, f.height, f.preset, f.varianten], [768, 1536, 'klein-9b', 1]);
  assert.equal(f.seedModus, 'fest');
  assert.equal(f.seed, 7);
});

test('Rückübernahme eines Listenauftrags liefert keinen Prompt und wird abgelehnt', () => {
  assert.equal(L.auftragInFormular({ kind: 'fullbody', liste: {} }, ARTEN), null);
});

test('mehr davon: gleicher Auftrag, neue Seeds', () => {
  const a = { kind: 'fullbody', prompt: 'x', seed: 100, seeds: [1, 2], name: 'n', style_ref: 'abc' };
  const n = L.nochmal(a, { eintrag: null }, 4);
  assert.deepEqual(n, { kind: 'fullbody', prompt: 'x', seed: -1, seeds: 4, name: 'n', style_ref: 'abc' });
  assert.deepEqual(a.seeds, [1, 2], 'das Original bleibt unverändert');
});

test('mehr davon bei einer Variante lässt seeds weg', () => {
  assert.equal('seeds' in L.nochmal({ kind: 'fullbody', prompt: 'x' }, {}, 1), false);
});

test('mehr davon bei Listenbildern wiederholt genau den Eintrag und erzwingt neue Bilder', () => {
  const n = L.nochmal({ kind: 'fullbody', liste: { from: 'a' }, seeds: 2 }, { eintrag: 'mira' }, 3);
  assert.deepEqual(n, { kind: 'fullbody', liste: { only: 'mira', force: true }, seed: -1, seeds: 3 });
});

test('Status wird für Anzeige übersetzt', () => {
  assert.deepEqual(L.statusInfo('laeuft'), { text: 'läuft', klasse: 'laeuft' });
  assert.equal(L.statusInfo('fehlgeschlagen').klasse, 'fehler');
  assert.equal(L.statusInfo('unbekannt').text, 'unbekannt');
  assert.equal(L.istAktiv('wartend'), true);
  assert.equal(L.istAktiv('laeuft'), true);
  assert.equal(L.istAktiv('fertig'), false);
});

test('Dauer und Zeitangaben lesbar', () => {
  assert.equal(L.dauerText(65), '1:05 min');
  assert.equal(L.dauerText(9), '9 s');
  assert.equal(L.dauerText(null), '');
  assert.equal(L.zeitText(1000, 1030), 'gerade eben');
  assert.equal(L.zeitText(1000, 1000 + 5 * 60), 'vor 5 min');
  assert.equal(L.zeitText(1000, 1000 + 3 * 3600), 'vor 3 h');
  assert.equal(L.zeitText(1000, 1000 + 2 * 86400), 'vor 2 Tagen');
  assert.equal(L.zeitText(1000, 1000 + 86400), 'vor 1 Tag');
});

test('Fortschritt in Prozent', () => {
  assert.equal(L.fortschritt({ bilder_fertig: 1, bilder_gesamt: 4 }), 25);
  assert.equal(L.fortschritt({ bilder_fertig: 0, bilder_gesamt: 0 }), 0);
});

test('Galerie: nur fertige Bilder, neueste Jobs zuerst, Reihenfolge im Job bleibt', () => {
  const jobs = [
    { id: 'a', erstellt: 100, bilder: [{ stamm: 'a1', status: 'fertig', url: '/a1' }, { stamm: 'a2', status: 'fehlgeschlagen' }] },
    { id: 'b', erstellt: 200, bilder: [{ stamm: 'b1', status: 'fertig', url: '/b1' }, { stamm: 'b2', status: 'fertig', url: '/b2' }, { stamm: 'b3', status: 'wartend' }] },
  ];
  const g = L.galerie(jobs);
  assert.deepEqual(g.map((e) => e.bild.stamm), ['b1', 'b2', 'a1']);
  assert.equal(g[0].job.id, 'b');
});

test('Galerie filtert nach Art', () => {
  const jobs = [
    { id: 'a', erstellt: 1, kind: 'token', bilder: [{ stamm: 'x', status: 'fertig', url: '/x' }] },
    { id: 'b', erstellt: 2, kind: 'fullbody', bilder: [{ stamm: 'y', status: 'fertig', url: '/y' }] },
  ];
  assert.deepEqual(L.galerie(jobs, 'token').map((e) => e.bild.stamm), ['x']);
  assert.equal(L.galerie(jobs, 'alle').length, 2);
});

test('Vorschau-Text fasst den Plan zusammen', () => {
  const t = L.vorschauKopf({ kind: 'fullbody', preset: 'klein-9b', groesse: [768, 1536], steps: 8, bilder: [{}, {}] });
  assert.match(t, /2 Bilder/);
  assert.match(t, /768×1536/);
  assert.match(t, /klein-9b/);
  assert.match(L.vorschauKopf({ kind: 'x', preset: 'p', groesse: [1, 1], steps: 1, bilder: [{}] }), /1 Bild\b/);
});

test('Dateiname zum Herunterladen', () => {
  assert.equal(L.downloadName({ stamm: 'dwarf-s5' }), 'dwarf-s5.png');
  assert.equal(L.downloadName({ stamm: 'dwarf', datei: 'dwarf.png' }), 'dwarf.png');
});

test('Fehlertext aus der API-Antwort', () => {
  assert.equal(L.fehlerText({ fehler: 'kaputt' }, 400), 'kaputt');
  assert.equal(L.fehlerText(null, 500), 'Fehler 500');
  assert.match(L.fehlerText(null, 401), /Token/);
});

test('HTML wird für die Anzeige maskiert', () => {
  assert.equal(L.esc('<b>"x"&</b>'), '&lt;b&gt;&quot;x&quot;&amp;&lt;/b&gt;');
  assert.equal(L.esc(null), '');
});

const messpunkt = (extra = {}) => ({
  t_ms: 61000, rss_mb: 8192, frei_mb: 1024, swap_mb: 2048, cpu_pct: 400,
  gesamt_mb: 16384, swap_gesamt_mb: 4096, kerne: 8, ...extra,
});

test('Speicherbalken: Prozess-Anteil und belegter Systemspeicher mit Warnstufe', () => {
  const [rss, system] = L.metrikBalken(messpunkt());
  assert.deepEqual(rss, { schluessel: 'rss', label: 'Prozess', prozent: 50, text: '8,0 GB', stufe: 'ok' });
  assert.deepEqual(system, { schluessel: 'system', label: 'System belegt', prozent: 94, text: '15,0 von 16,0 GB', stufe: 'kritisch' });
});

test('Swap: schon ab 1 MB eine Warnung', () => {
  const balken = L.metrikBalken(messpunkt());
  const swap = balken.find((b) => b.schluessel === 'swap');
  assert.deepEqual(swap, { schluessel: 'swap', label: 'Swap', prozent: 50, text: '2,0 von 4,0 GB', stufe: 'kritisch' });
  assert.equal(L.metrikBalken(messpunkt({ swap_mb: 0 })).find((b) => b.schluessel === 'swap').stufe, 'ok');
  assert.equal(L.metrikBalken(messpunkt({ swap_mb: 100 })).find((b) => b.schluessel === 'swap').stufe, 'warn');
});

test('ohne Swap-Größe entsteht kein NaN und ein belegter Swap bleibt sichtbar', () => {
  const leer = L.metrikBalken(messpunkt({ swap_mb: 0, swap_gesamt_mb: 0 })).find((b) => b.schluessel === 'swap');
  assert.equal(leer.prozent, 0);
  assert.equal(leer.stufe, 'ok');
  const belegt = L.metrikBalken(messpunkt({ swap_mb: 512, swap_gesamt_mb: 0 })).find((b) => b.schluessel === 'swap');
  assert.equal(belegt.prozent, 100);
  assert.equal(belegt.stufe, 'kritisch');
});

test('Spitzenpunkt: Höchstwerte auf der Skala des letzten Messpunkts, ohne Proben nichts', () => {
  const z = {
    spitze_rss_mb: 9000, min_frei_mb: 150, max_swap_mb: 4000, spitze_cpu_pct: 790,
    aktuell: messpunkt({ rss_mb: 100, frei_mb: 5000, swap_mb: 0, cpu_pct: 1 }),
  };
  assert.deepEqual(L.spitzenPunkt(z), messpunkt({ rss_mb: 9000, frei_mb: 150, swap_mb: 4000, cpu_pct: 790, swap_ein_mb_s: 0, swap_aus_mb_s: 0 }));
  const mitRate = L.spitzenPunkt({ ...z, spitze_swap_ein_mb_s: 12, spitze_swap_aus_mb_s: 80 });
  assert.deepEqual([mitRate.swap_ein_mb_s, mitRate.swap_aus_mb_s], [12, 80]);
  assert.equal(L.spitzenPunkt({ ...z, aktuell: null }), null);
});

test('Phasennamen werden lesbar, Unbekanntes bleibt wie es ist', () => {
  assert.equal(L.phasenText('erzeugen'), 'Bild erzeugen');
  assert.equal(L.phasenText('referenz'), 'Referenz vorbereiten');
  assert.equal(L.phasenText('freistellen'), 'Freistellen');
  assert.equal(L.phasenText('irgendwas'), 'irgendwas');
  assert.equal(L.phasenText(null), '');
});

test('Auslagerungsbalken: Swap-Rate statt CPU, rot ab 20 MB/s', () => {
  const rate = (ein, aus) => L.metrikBalken(messpunkt({ swap_ein_mb_s: ein, swap_aus_mb_s: aus }));
  const a = rate(0, 40).find((b) => b.schluessel === 'auslagerung');
  assert.deepEqual(a, { schluessel: 'auslagerung', label: 'Auslagerung', prozent: 20, text: 'ein 0,0 · aus 40,0 MB/s', stufe: 'kritisch' });
  assert.equal(rate(0, 0).find((b) => b.schluessel === 'auslagerung').stufe, 'ok');
  assert.equal(rate(3, 0).find((b) => b.schluessel === 'auslagerung').stufe, 'warn');
  assert.equal(rate(0, 0).some((b) => b.schluessel === 'cpu'), false, 'CPU liegt bei GPU-Läufen immer ~0');
  // Ältere Antworten kennen die Raten nicht.
  const alt = messpunkt(); delete alt.swap_ein_mb_s; delete alt.swap_aus_mb_s;
  assert.equal(L.metrikBalken(alt).find((b) => b.schluessel === 'auslagerung').prozent, 0);
});

test('Referenz-Obergrenze geht nur mit Referenz und gewähltem Wert in den Auftrag', () => {
  const mitRef = { id: 'abc', modus: 'inhalt' };
  assert.equal(L.auftragAusFormular(form({ stilref: mitRef, refMaxPx: '512' }), ARTEN).ref_max_px, 512);
  assert.equal('ref_max_px' in L.auftragAusFormular(form({ stilref: mitRef, refMaxPx: '' }), ARTEN), false, 'Standard');
  assert.equal('ref_max_px' in L.auftragAusFormular(form({ stilref: mitRef }), ARTEN), false, 'Feld fehlt ganz');
  assert.equal('ref_max_px' in L.auftragAusFormular(form({ stilref: null, refMaxPx: '512' }), ARTEN), false, 'ohne Referenz sinnlos');
});

test('Auftrag übernehmen füllt die Referenz-Obergrenze wieder ein', () => {
  const a = { kind: 'fullbody', prompt: 'x', refs: ['abc'], ref_max_px: 512 };
  assert.equal(L.auftragInFormular(a, ARTEN, null).refMaxPx, '512');
  assert.equal(L.auftragInFormular({ kind: 'fullbody', prompt: 'x' }, ARTEN, null).refMaxPx, '');
});
