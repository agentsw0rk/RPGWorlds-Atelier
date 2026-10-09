// RPGWorlds Atelier — die Oberfläche der flux2-api. Reine Logik steckt in logik.js
// (getestet mit `node --test web/`); hier hängt sie am DOM und an der API.
'use strict';

(function () {
  const L = window.Logik;
  const $ = (sel, wurzel = document) => wurzel.querySelector(sel);
  const $$ = (sel, wurzel = document) => [...wurzel.querySelectorAll(sel)];

  const ART_NAMEN = { token: 'Token', location: 'Ort', portrait: 'Porträt', fullbody: 'Ganzkörper' };
  const ART_HINWEIS = {
    token: 'freigestellt, auf Sockel',
    location: 'Querformat-Illustration',
    portrait: 'Kopf und Schultern',
    fullbody: 'Figur in Szene',
  };
  const artName = (k) => ART_NAMEN[k] || k;

  const S = {
    arten: [],
    kind: 'fullbody',
    kurz: [],            // GET /jobs
    jobs: new Map(),     // id -> GET /jobs/{id}
    metriken: new Map(), // id -> zusammenfassung aus GET /jobs/{id}/metrics?kurz=1 (null = gibt es nicht)
    metrikStand: new Map(), // id -> Jobstatus, bei dem metriken zuletzt geholt wurde
    tab: 'erstellen',
    galerieFilter: 'alle',
    listenArt: 'fullbody',
    listen: {},          // art -> GET /listen/{art}
    listeVeraltet: true,
    auswahl: new Set(),
    stilref: null,       // { id, vorschau }
    verbunden: null,
    polling: false,
    timer: null,
    tokenWartet: null,
  };

  // ---------------------------------------------------------------- Token & API

  const tokenSpeicher = {
    lesen() { try { return localStorage.getItem('flux2-token') || ''; } catch { return ''; } },
    setzen(wert) { try { localStorage.setItem('flux2-token', wert); } catch { /* privater Modus */ } },
  };

  function tokenAbfragen() {
    if (S.tokenWartet) return S.tokenWartet;
    const dialog = $('#tokenDialog');
    S.tokenWartet = new Promise((fertig) => {
      const ende = (speichern) => {
        if (speichern) {
          tokenSpeicher.setzen($('#tokenFeld').value.trim());
          blobCache.clear();
        }
        if (dialog.open) dialog.close();
        S.tokenWartet = null;
        fertig();
      };
      $('#tokenForm').onsubmit = (e) => { e.preventDefault(); ende(true); };
      $('#tokenAbbruch').onclick = () => ende(false);
      dialog.oncancel = () => ende(false);
      $('#tokenFeld').value = tokenSpeicher.lesen();
      if (!dialog.open) dialog.showModal();
      $('#tokenFeld').focus();
    });
    return S.tokenWartet;
  }

  async function api(pfad, { methode = 'GET', json, body, nochmal = true } = {}) {
    const kopf = {};
    const token = tokenSpeicher.lesen();
    if (token) kopf.Authorization = `Bearer ${token}`;
    let inhalt = body;
    if (json !== undefined) {
      inhalt = JSON.stringify(json);
      kopf['Content-Type'] = 'application/json';
    }
    let antwort;
    try {
      antwort = await fetch(pfad, { method: methode, headers: kopf, body: inhalt });
    } catch {
      throw Object.assign(new Error('Keine Verbindung zur API.'), { netz: true });
    }
    if (antwort.status === 401 && nochmal) {
      await tokenAbfragen();
      return api(pfad, { methode, json, body, nochmal: false });
    }
    let daten = null;
    if ((antwort.headers.get('content-type') || '').includes('json')) {
      daten = await antwort.json().catch(() => null);
    }
    if (!antwort.ok) {
      throw Object.assign(new Error(L.fehlerText(daten, antwort.status)), { status: antwort.status, daten });
    }
    return daten;
  }

  // ---------------------------------------------------------------- Bilder (lazy)

  // Ohne Token reicht die URL (der Browser cached sie). Mit Token kann <img> keinen
  // Authorization-Header senden — dann wird das Bild per fetch geholt und als blob gezeigt.
  const blobCache = new Map();
  function bildQuelle(url) {
    const token = tokenSpeicher.lesen();
    if (!token) return Promise.resolve(url);
    if (!blobCache.has(url)) {
      blobCache.set(url, fetch(url, { headers: { Authorization: `Bearer ${token}` } })
        .then((r) => { if (!r.ok) throw new Error(String(r.status)); return r.blob(); })
        .then((b) => URL.createObjectURL(b))
        .catch((e) => { blobCache.delete(url); throw e; }));
    }
    return blobCache.get(url);
  }

  const beobachter = 'IntersectionObserver' in window
    ? new IntersectionObserver((eintraege) => {
      for (const e of eintraege) {
        if (!e.isIntersecting) continue;
        beobachter.unobserve(e.target);
        laden(e.target);
      }
    }, { rootMargin: '300px' })
    : null;

  function laden(img) {
    bildQuelle(img.dataset.src)
      .then((q) => { img.src = q; })
      .catch(() => img.classList.add('defekt'));
  }

  function lazyBilder(wurzel) {
    for (const img of $$('img[data-src]', wurzel)) {
      if (img.dataset.beobachtet) continue;
      img.dataset.beobachtet = '1';
      if (beobachter) beobachter.observe(img); else laden(img);
    }
  }

  // ---------------------------------------------------------------- Kleinkram

  function meldung(text, art = '') {
    const el = document.createElement('div');
    el.className = `meldung ${art}`;
    el.textContent = text;
    $('#meldungen').append(el);
    setTimeout(() => el.remove(), art === 'fehler' ? 8000 : 4000);
  }

  function bestaetigen(text, ja = 'Ja') {
    const dialog = $('#bestaetigung');
    $('#bestaetigungText').textContent = text;
    $('#bestaetigungJa').textContent = ja;
    dialog.returnValue = 'nein';
    dialog.showModal();
    return new Promise((fertig) => {
      dialog.addEventListener('close', () => fertig(dialog.returnValue === 'ja'), { once: true });
    });
  }

  function htmlZuElement(html) {
    const t = document.createElement('template');
    t.innerHTML = html.trim();
    return t.content.firstElementChild;
  }

  // Gleicht eine Liste von Karten mit den Daten ab, ohne unveränderte neu zu bauen —
  // sonst flackern Vorschaubilder bei jedem Poll.
  function abgleichen(behaelter, items, schluessel, signatur, bauen) {
    const alt = new Map($$(':scope > [data-key]', behaelter).map((n) => [n.dataset.key, n]));
    let vorher = null;
    for (const item of items) {
      const k = String(schluessel(item));
      const sig = signatur(item);
      let knoten = alt.get(k);
      if (!knoten || knoten.dataset.sig !== sig) {
        const neu = bauen(item);
        neu.dataset.key = k;
        neu.dataset.sig = sig;
        if (knoten) knoten.replaceWith(neu);
        knoten = neu;
        lazyBilder(neu);
      }
      alt.delete(k);
      const soll = vorher ? vorher.nextSibling : behaelter.firstChild;
      if (knoten !== soll) behaelter.insertBefore(knoten, soll);
      vorher = knoten;
    }
    for (const rest of alt.values()) rest.remove();
  }

  // ---------------------------------------------------------------- Tabs

  function tabWechseln(name) {
    S.tab = name;
    for (const b of $$('.tabs [role="tab"]')) b.setAttribute('aria-selected', String(b.dataset.tab === name));
    for (const s of $$('.bereich')) s.hidden = s.id !== `tab-${name}`;
    if (name === 'jobs') renderJobs();
    if (name === 'galerie') renderGalerie();
    if (name === 'listen') listeLaden(S.listenArt);
    try { history.replaceState(null, '', `#${name}`); } catch { /* egal */ }
  }

  // ---------------------------------------------------------------- Erstellen

  function artWaehlen(kind, { groesseBehalten = false } = {}) {
    const vorher = S.arten.find((a) => a.kind === S.kind);
    const art = S.arten.find((a) => a.kind === kind);
    if (!art) return;
    // Größe nur auf den Standard der neuen Art setzen, wenn der Nutzer sie nicht angepasst hat.
    const breite = $('#breite');
    const hoehe = $('#hoehe');
    const leer = !breite.value || !hoehe.value;
    const unveraendert = !vorher || leer || (Number(breite.value) === vorher.width && Number(hoehe.value) === vorher.height);
    if (!groesseBehalten && unveraendert) {
      breite.value = art.width;
      hoehe.value = art.height;
    }
    S.kind = kind;
    for (const b of $$('#artWahl .segment')) b.setAttribute('aria-pressed', String(b.dataset.kind === kind));
    $('#zeilePose').hidden = kind !== 'token';
    $('#zeileLarge').hidden = kind !== 'location';
  }

  function formularLesen() {
    const modus = ($('input[name="seedModus"]:checked') || {}).value || 'wuerfeln';
    return {
      kind: S.kind,
      prompt: $('#prompt').value,
      varianten: Number($('#varianten').value),
      seedModus: modus,
      seed: Number($('#seed').value),
      preset: $('#preset').value,
      width: Number($('#breite').value),
      height: Number($('#hoehe').value),
      steps: $('#steps').value,
      cfg: $('#cfg').value,
      guidance: $('#guidance').value,
      refMaxPx: $('#refMaxPx').value,
      name: $('#name').value,
      style: $('#style').value,
      pose: $('#pose').value,
      large: $('#large').checked,
      freistellen: $('#freistellen').value,
      stilref: S.stilref ? { id: S.stilref.id, modus: ($('input[name="refModus"]:checked') || {}).value || 'stil' } : null,
    };
  }

  function formularSchreiben(f) {
    artWaehlen(f.kind, { groesseBehalten: true });
    $('#prompt').value = f.prompt;
    $('#varianten').value = f.varianten;
    for (const r of $$('input[name="seedModus"]')) r.checked = r.value === f.seedModus;
    $('#seed').value = f.seed;
    $('#seed').disabled = f.seedModus !== 'fest';
    $('#preset').value = f.preset;
    $('#breite').value = f.width;
    $('#hoehe').value = f.height;
    $('#steps').value = f.steps;
    $('#cfg').value = f.cfg;
    $('#guidance').value = f.guidance;
    // Ein Wert, den die Liste nicht kennt (per API gesetzt), wird als eigene Option gezeigt.
    if (f.refMaxPx && ![...$('#refMaxPx').options].some((o) => o.value === f.refMaxPx)) {
      $('#refMaxPx').add(new Option(`${f.refMaxPx} px`, f.refMaxPx));
    }
    $('#refMaxPx').value = f.refMaxPx || '';
    $('#name').value = f.name;
    $('#style').value = f.style;
    $('#pose').value = f.pose;
    $('#large').checked = f.large;
    $('#freistellen').value = f.freistellen;
    if (f.stilref) {
      refZeigen({ id: f.stilref.id, vorschau: null });
      for (const r of $$('input[name="refModus"]')) r.checked = r.value === f.stilref.modus;
    } else {
      refEntfernen();
    }
    // Weitere Einstellungen aufklappen, wenn dort etwas Abweichendes steht.
    const art = S.arten.find((a) => a.kind === f.kind) || {};
    const abweichend = f.preset !== L.STANDARD_PRESET || f.steps || f.cfg || f.guidance || f.refMaxPx || f.name || f.style || f.pose
      || f.large || f.freistellen !== 'auto' || f.width !== art.width || f.height !== art.height;
    if (abweichend) $('details.mehr').open = true;
  }

  const FORM_KEY = 'flux2-form';
  function formularMerken() {
    try {
      const f = formularLesen();
      f.stilref = null; // hochgeladene Bilder lassen sich nach dem Neuladen nicht mehr anzeigen
      localStorage.setItem(FORM_KEY, JSON.stringify(f));
    } catch { /* privater Modus */ }
  }
  function formularErinnern() {
    try {
      const roh = localStorage.getItem(FORM_KEY);
      if (roh) formularSchreiben({ ...L.auftragInFormular({ kind: 'fullbody', prompt: '' }, S.arten), ...JSON.parse(roh) });
    } catch { /* kaputter Eintrag */ }
  }

  function hinweis(text) {
    const el = $('#formHinweis');
    el.textContent = text || '';
    el.hidden = !text;
  }

  // -- Referenzbild
  function refZeigen({ id, vorschau }) {
    S.stilref = { id, vorschau };
    $('#ablageLeer').hidden = true;
    $('#ablageVoll').hidden = false;
    const bild = $('#ablageBild');
    if (vorschau) { bild.src = vorschau; bild.hidden = false; } else { bild.removeAttribute('src'); bild.hidden = true; }
  }

  function refEntfernen() {
    if (S.stilref && S.stilref.vorschau) URL.revokeObjectURL(S.stilref.vorschau);
    S.stilref = null;
    $('#ablageLeer').hidden = false;
    $('#ablageVoll').hidden = true;
    $('#datei').value = '';
  }

  async function dateiHochladen(datei) {
    if (!datei || !datei.type.startsWith('image/')) { meldung('Das ist kein Bild.', 'fehler'); return; }
    try {
      const { id } = await api('/uploads', { methode: 'POST', body: datei });
      refZeigen({ id, vorschau: URL.createObjectURL(datei) });
      meldung('Referenzbild hochgeladen.', 'ok');
    } catch (e) {
      meldung(`Upload fehlgeschlagen: ${e.message}`, 'fehler');
    }
  }

  // -- Vorschau & Start
  function auftragPruefen() {
    hinweis('');
    const f = formularLesen();
    const fehler = L.formularFehler(f);
    if (fehler) { hinweis(fehler); return null; }
    return L.auftragAusFormular(f, S.arten);
  }

  function apiFehler(e) {
    // Eine verschwundene Referenz (z. B. Daten gelöscht) nicht stehen lassen.
    if (/Upload '.*' gibt es nicht/.test(e.message)) refEntfernen();
    hinweis(e.message);
  }

  async function vorschau() {
    const auftrag = auftragPruefen();
    if (!auftrag) return;
    try {
      const plan = await api('/jobs?dry_run=1', { methode: 'POST', json: auftrag });
      const ziel = $('#vorschau');
      ziel.innerHTML = `<div class="vorschau-kopf">${L.esc(L.vorschauKopf(plan))}</div>`
        + '<ul class="plan">'
        + plan.bilder.map((b) => `<li>
            <div class="kopfzeile">
              <span class="datei">${L.esc(b.stamm)}.png</span>
              <span class="marke-klein">Seed ${L.esc(b.seed)}</span>
              ${b.freigestellt ? '<span class="marke-klein">freigestellt</span>' : ''}
              ${b.referenzen ? `<span class="marke-klein">${L.esc(b.referenzen)} Referenz</span>` : ''}
              ${b.uebersprungen ? '<span class="marke-klein">vorhanden</span>' : ''}
            </div>
            <details><summary>Prompt</summary><pre>${L.esc(b.prompt)}</pre></details>
          </li>`).join('')
        + '</ul>';
    } catch (e) { apiFehler(e); }
  }

  async function starten(ereignis) {
    ereignis.preventDefault();
    const auftrag = auftragPruefen();
    if (!auftrag) return;
    const knopf = $('#startKnopf');
    knopf.disabled = true;
    try {
      const job = await api('/jobs', { methode: 'POST', json: auftrag });
      meldung(`Job ${job.id} gestartet — ${job.bilder.length} ${job.bilder.length === 1 ? 'Bild' : 'Bilder'}.`, 'ok');
      await aktualisieren();
      tabWechseln('jobs');
    } catch (e) { apiFehler(e); } finally { knopf.disabled = false; }
  }

  // ---------------------------------------------------------------- Jobs

  const bilderFertig = (job) => job.bilder.filter((b) => b.status === 'fertig' || b.status === 'vorhanden').length;

  function thumbHtml(job, bild, index) {
    const seed = `<span class="seed">Seed ${L.esc(bild.seed)}</span>`;
    if (bild.status === 'fertig' && bild.url) {
      return `<button class="thumb" data-aktion="oeffnen" data-index="${index}" title="${L.esc(bild.stamm)}">
        <img data-src="${L.esc(bild.url)}" alt="${L.esc(bild.stamm)}">${seed}</button>`;
    }
    if (bild.status === 'laeuft') return `<div class="thumb laeuft leer-platz"><div class="platz"><b>◔</b>erzeugt …</div>${seed}</div>`;
    if (bild.status === 'fehlgeschlagen') return `<div class="thumb fehler leer-platz" title="${L.esc(bild.fehler || '')}"><div class="platz"><b>✕</b>fehlgeschlagen</div>${seed}</div>`;
    if (bild.status === 'abgebrochen') return `<div class="thumb leer-platz"><div class="platz"><b>–</b>abgebrochen</div>${seed}</div>`;
    return `<div class="thumb leer-platz"><div class="platz"><b>⋯</b>wartet</div>${seed}</div>`;
  }

  // Balken für Speicher, Swap und CPU. Läuft der Job: aktueller Stand; sonst die Spitzenwerte.
  function metrikHtml(k) {
    const z = S.metriken.get(k.id);
    if (!z) return '';
    const lauf = k.status === 'laeuft';
    const punkt = lauf ? z.aktuell : L.spitzenPunkt(z);
    if (!punkt) return '';
    const phase = lauf ? L.phasenText(z.letzte_phase) : '';
    const kopf = lauf ? `Live${phase ? ` · ${phase}` : ''}` : 'Spitzenwerte des Laufs';
    const zeilen = L.metrikBalken(punkt).map((b) => `<div class="metrik ${b.stufe}" title="${L.esc(b.label)}: ${L.esc(b.text)}">
        <span class="metrik-label">${L.esc(b.label)}</span>
        <div class="balken" role="meter" aria-label="${L.esc(b.label)}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${b.prozent}"><i style="width:${b.prozent}%"></i></div>
        <span class="metrik-text">${L.esc(b.text)}</span>
      </div>`).join('');
    return `<div class="metriken-kopf">${L.esc(kopf)}</div>${zeilen}`;
  }

  // Die Balken werden an Ort und Stelle getauscht, nicht die ganze Karte neu gebaut:
  // sonst flackern die Vorschaubilder jede Sekunde.
  function metrikenZeichnen() {
    for (const k of S.kurz) {
      const ziel = $(`#jobListe [data-id="${CSS.escape(k.id)}"] .metriken`);
      if (!ziel) continue;
      const html = metrikHtml(k);
      ziel.hidden = !html;
      if (ziel.dataset.html !== html) { ziel.innerHTML = html; ziel.dataset.html = html; }
    }
  }

  async function metrikenLaden(jobs) {
    await Promise.all(jobs.map(async (k) => {
      // Laufende fragen wir bei jedem Durchgang, beendete einmal nach dem Statuswechsel,
      // wartende gar nicht.
      const laeuft = k.status === 'laeuft';
      if (!laeuft && (S.metrikStand.get(k.id) === k.status || L.istAktiv(k.status))) return;
      try {
        S.metriken.set(k.id, (await api(`/jobs/${k.id}/metrics?kurz=1`)).zusammenfassung);
        S.metrikStand.set(k.id, k.status);
      } catch (e) {
        if (e.status === 404) { S.metriken.set(k.id, null); S.metrikStand.set(k.id, k.status); }
      }
    }));
  }

  function jobKarte(k) {
    const job = S.jobs.get(k.id);
    const info = L.statusInfo(k.status);
    const aktiv = L.istAktiv(k.status);
    const prozent = L.fortschritt(k);
    const prompt = job && typeof job.auftrag.prompt === 'string';
    const dauer = job && job.beendet && job.gestartet ? ` · ${L.dauerText(job.beendet - job.gestartet)}` : '';
    const karte = htmlZuElement(`<article class="karte job" data-id="${L.esc(k.id)}">
      <div class="job-kopf">
        <span class="badge ${info.klasse}">${L.esc(info.text)}</span>
        <span class="job-titel">${L.esc(k.titel)}</span>
        <span class="job-meta">${L.esc(artName(k.kind))} · ${L.esc(L.zeitText(k.erstellt, Date.now() / 1000))}${L.esc(dauer)}</span>
      </div>
      <div class="balken ${k.status === 'laeuft' ? 'laeuft' : ''}" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${prozent}"><i style="width:${prozent}%"></i></div>
      <div class="job-meldung">${L.esc(k.meldung)} · ${k.bilder_fertig}/${k.bilder_gesamt}</div>
      <div class="metriken" hidden></div>
      ${job && job.fehler ? `<div class="job-fehler">${L.esc(job.fehler)}</div>` : ''}
      <div class="thumbs">${job ? job.bilder.map((b, i) => thumbHtml(job, b, i)).join('') : ''}</div>
      <div class="job-fuss">
        ${aktiv ? '<button class="sekundaer gefahr" data-aktion="abbrechen">Abbrechen</button>' : '<button class="schlicht" data-aktion="entfernen">Entfernen</button>'}
        ${prompt && !aktiv ? '<button class="sekundaer" data-aktion="wiederholen">Mehr davon</button><button class="sekundaer" data-aktion="uebernehmen">Auftrag übernehmen</button>' : ''}
      </div>
    </article>`);
    return karte;
  }

  function renderJobs() {
    const liste = $('#jobListe');
    abgleichen(liste, S.kurz, (k) => k.id,
      (k) => {
        const job = S.jobs.get(k.id);
        return JSON.stringify([k.status, k.meldung, k.bilder_fertig, k.titel, Math.floor(Date.now() / 60000),
          job ? job.bilder.map((b) => [b.status, b.url, b.fehler]) : 0, job ? job.fehler : 0]);
      },
      jobKarte);
    $('#jobLeer').hidden = S.kurz.length > 0;
    metrikenZeichnen();
  }

  async function jobAktion(aktion, id, knopf) {
    const job = S.jobs.get(id);
    try {
      if (aktion === 'abbrechen') {
        await api(`/jobs/${id}`, { methode: 'DELETE' });
        meldung('Abbruch angefordert — nach dem aktuellen Bild.');
      } else if (aktion === 'entfernen') {
        await api(`/jobs/${id}`, { methode: 'DELETE' });
      } else if (aktion === 'wiederholen' && job) {
        const a = job.auftrag;
        const anzahl = Array.isArray(a.seeds) ? a.seeds.length : (a.seeds || 1);
        const neu = await api('/jobs', { methode: 'POST', json: L.nochmal(a, {}, anzahl) });
        meldung(`Job ${neu.id} gestartet.`, 'ok');
      } else if (aktion === 'uebernehmen' && job) {
        auftragUebernehmen(job.auftrag);
        return;
      } else if (aktion === 'oeffnen' && job) {
        const eintraege = L.galerie([job]);
        const stamm = job.bilder[Number(knopf.dataset.index)].stamm;
        lightboxOeffnen(eintraege.map(galerieEintrag), Math.max(0, eintraege.findIndex((e) => e.bild.stamm === stamm)));
        return;
      }
      await aktualisieren();
    } catch (e) { meldung(e.message, 'fehler'); }
  }

  async function beendeteEntfernen() {
    // Frisch holen: der Polling-Stand kann einen Job noch als laufend führen, der gerade fertig wurde.
    let beendet;
    try { beendet = (await api('/jobs')).jobs.filter((k) => !L.istAktiv(k.status)); } catch (e) { meldung(e.message, 'fehler'); return; }
    if (!beendet.length) { meldung('Keine beendeten Jobs.'); return; }
    if (!await bestaetigen(`${beendet.length} beendete Jobs samt ihren Bildern entfernen? Bilder aus Listen bleiben erhalten.`, 'Entfernen')) return;
    for (const k of beendet) {
      try { await api(`/jobs/${k.id}`, { methode: 'DELETE' }); } catch (e) { meldung(e.message, 'fehler'); }
    }
    await aktualisieren();
  }

  function auftragUebernehmen(auftrag) {
    const f = L.auftragInFormular(auftrag, S.arten);
    if (!f) { meldung('Listenaufträge haben keinen eigenen Prompt.', 'fehler'); return; }
    if (lightbox.open) lightbox.close();
    formularSchreiben(f);
    formularMerken();
    tabWechseln('erstellen');
    $('#prompt').focus();
    meldung('Auftrag übernommen — Seeds sind auf „zufällig“ gestellt, wenn der Auftrag keinen festen hatte.');
  }

  // ---------------------------------------------------------------- Galerie

  function galerieEintrag({ job, bild }) {
    const a = job.auftrag;
    return {
      url: bild.url,
      titel: bild.stamm,
      zeilen: [
        ['Art', artName(job.kind)],
        ['Seed', bild.seed],
        ['Größe', `${job.groesse[0]}×${job.groesse[1]}`],
        ['Modell', job.preset],
        ['Steps', job.steps],
        ['Dauer', L.dauerText(bild.dauer_s) || '–'],
        ['Erstellt', L.zeitText(job.erstellt, Date.now() / 1000)],
      ],
      prompt: bild.prompt,
      download: L.downloadName(bild),
      nochmal: (anzahl) => neuerJob(L.nochmal(a, bild, anzahl)),
      uebernehmen: typeof a.prompt === 'string' ? () => auftragUebernehmen(a) : null,
    };
  }

  function renderGalerie() {
    const arten = ['alle', ...S.arten.map((a) => a.kind)];
    const filter = $('#galerieFilter');
    if (filter.children.length !== arten.length) {
      filter.innerHTML = arten.map((k) => `<button class="chip" data-art="${k}" aria-pressed="${k === S.galerieFilter}">${k === 'alle' ? 'Alle' : artName(k)}</button>`).join('');
    }
    for (const b of $$('.chip', filter)) b.setAttribute('aria-pressed', String(b.dataset.art === S.galerieFilter));

    const eintraege = L.galerie([...S.jobs.values()], S.galerieFilter);
    abgleichen($('#galerieRaster'), eintraege, (e) => `${e.job.id}/${e.bild.stamm}`,
      (e) => e.bild.url,
      (e) => htmlZuElement(`<button class="kachel" data-job="${L.esc(e.job.id)}" data-stamm="${L.esc(e.bild.stamm)}">
        <span class="bild"><img data-src="${L.esc(e.bild.url)}" alt="${L.esc(e.bild.stamm)}"></span>
        <span class="unter">${L.esc(e.bild.stamm)}</span></button>`));
    $('#galerieLeer').hidden = eintraege.length > 0;
  }

  // ---------------------------------------------------------------- Listen

  const ListeOpt = () => ({
    varianten: Number($('#listeVarianten').value) || 1,
    preset: $('#listePreset').value,
    zufall: $('#listeZufall').checked,
    force: $('#listeForce').checked,
  });

  async function listeLaden(art, { still = false } = {}) {
    S.listenArt = art;
    renderListenArt();
    if (S.listen[art] && !S.listeVeraltet) { renderListe(); return; }
    try {
      S.listen[art] = await api(`/listen/${art}`);
      S.listeVeraltet = false;
    } catch (e) {
      if (!still) meldung(`Liste nicht ladbar: ${e.message}`, 'fehler');
      return;
    }
    renderListe();
  }

  function renderListenArt() {
    const ziel = $('#listenArt');
    if (ziel.children.length !== S.arten.length) {
      ziel.innerHTML = S.arten.map((a) => `<button class="chip" data-art="${a.kind}">${artName(a.kind)}</button>`).join('');
    }
    for (const b of $$('.chip', ziel)) b.setAttribute('aria-pressed', String(b.dataset.art === S.listenArt));
  }

  function sichtbareEintraege() {
    const liste = S.listen[S.listenArt];
    if (!liste) return [];
    const q = $('#listeSuche').value.trim().toLowerCase();
    return liste.eintraege.filter((e) => !q || `${e.slug} ${e.name} ${e.beschreibung} ${e.prompt}`.toLowerCase().includes(q));
  }

  function renderListe() {
    const liste = S.listen[S.listenArt];
    if (!liste) return;
    const sichtbar = sichtbareEintraege();
    const art = S.listenArt;
    abgleichen($('#listeRaster'), sichtbar, (e) => `${art}/${e.slug}`,
      (e) => JSON.stringify([e.bilder, S.auswahl.has(`${art}/${e.slug}`)]),
      (e) => {
        const gewaehlt = S.auswahl.has(`${art}/${e.slug}`);
        return htmlZuElement(`<article class="eintrag ${gewaehlt ? 'gewaehlt' : ''}" data-slug="${L.esc(e.slug)}">
          <input type="checkbox" ${gewaehlt ? 'checked' : ''} aria-label="${L.esc(e.name)} auswählen">
          <div>
            <div class="name">${L.esc(e.name)} ${e.bilder.length ? `<span class="badge fertig">${e.bilder.length} ${e.bilder.length === 1 ? 'Bild' : 'Bilder'}</span>` : '<span class="badge">fehlt</span>'}</div>
            <div class="beschr">${L.esc(e.beschreibung)}</div>
            <details><summary>Prompt</summary><pre>${L.esc(e.prompt)}</pre></details>
          </div>
          ${e.bilder.length ? `<div class="thumbs">${e.bilder.slice(0, 8).map((d, i) => `<button class="thumb" data-aktion="bild" data-index="${i}" title="${L.esc(d)}"><img data-src="/bibliothek/${art}/${L.esc(d)}" alt="${L.esc(d)}"></button>`).join('')}</div>` : ''}
        </article>`);
      });
    $('#listeLeer').hidden = sichtbar.length > 0;
    const gewaehlt = [...S.auswahl].filter((k) => k.startsWith(`${art}/`)).length;
    const fehlend = liste.eintraege.filter((e) => !e.bilder.length).length;
    $('#listeInfo').textContent = `${liste.eintraege.length} Einträge · ${fehlend} ohne Bild · ${gewaehlt} gewählt`;
    $('#listeAuswahl').disabled = gewaehlt === 0;
    $('#listeAuswahl').textContent = gewaehlt ? `Auswahl starten (${gewaehlt})` : 'Auswahl starten';
  }

  async function neuerJob(auftrag) {
    try {
      const job = await api('/jobs', { methode: 'POST', json: auftrag });
      meldung(`Job ${job.id} gestartet — ${job.bilder.length} ${job.bilder.length === 1 ? 'Bild' : 'Bilder'}.`, 'ok');
      await aktualisieren();
      return job;
    } catch (e) { meldung(e.message, 'fehler'); return null; }
  }

  function listenAuftrag(art, slug, opt) {
    const a = { kind: art, liste: { only: slug, force: opt.force }, seed: opt.zufall ? -1 : 42 };
    if (opt.varianten > 1) a.seeds = opt.varianten;
    if (opt.preset !== L.STANDARD_PRESET) a.preset = opt.preset;
    return a;
  }

  async function listeStarten(slugs) {
    if (!slugs.length) { meldung('Nichts zu starten — alle Einträge haben schon ein Bild.'); return; }
    const opt = ListeOpt();
    if (slugs.length * opt.varianten > 12
      && !await bestaetigen(`Das legt ${slugs.length} Jobs mit zusammen ${slugs.length * opt.varianten} Bildern an. Fortfahren?`, 'Starten')) return;
    let n = 0;
    for (const slug of slugs) {
      try {
        await api('/jobs', { methode: 'POST', json: listenAuftrag(S.listenArt, slug, opt) });
        n += 1;
      } catch (e) { meldung(`${slug}: ${e.message}`, 'fehler'); break; }
    }
    if (n) meldung(`${n} ${n === 1 ? 'Job' : 'Jobs'} gestartet.`, 'ok');
    await aktualisieren();
    if (n) tabWechseln('jobs');
  }

  function listeBildOeffnen(art, eintrag, index) {
    const eintraege = eintrag.bilder.map((datei) => ({
      url: `/bibliothek/${art}/${datei}`,
      titel: datei,
      zeilen: [['Art', artName(art)], ['Eintrag', eintrag.name], ['Datei', datei]],
      prompt: eintrag.prompt,
      download: datei,
      nochmal: (anzahl) => neuerJob({ ...listenAuftrag(art, eintrag.slug, { ...ListeOpt(), varianten: anzahl, zufall: true, force: true }) }),
      uebernehmen: () => {
        formularSchreiben({ ...L.auftragInFormular({ kind: art, prompt: eintrag.prompt }, S.arten), name: eintrag.slug });
        formularMerken();
        if (lightbox.open) lightbox.close();
        tabWechseln('erstellen');
      },
    }));
    lightboxOeffnen(eintraege, index);
  }

  // ---------------------------------------------------------------- Lightbox

  const lightbox = $('#lightbox');
  let lbListe = [];
  let lbIndex = 0;

  function lightboxOeffnen(liste, index) {
    if (!liste.length) return;
    lbListe = liste;
    lbIndex = index;
    lbZeigen();
    if (!lightbox.open) lightbox.showModal();
  }

  function lbZeigen() {
    const e = lbListe[lbIndex];
    const bild = $('#lbBild');
    bild.removeAttribute('src');
    bild.alt = e.titel;
    bildQuelle(e.url).then((q) => {
      if (lbListe[lbIndex] !== e) return; // inzwischen weitergeblättert
      bild.src = q;
      const dl = $('#lbDownload');
      dl.href = q;
      dl.download = e.download;
    }).catch(() => meldung('Bild nicht ladbar.', 'fehler'));
    $('#lbTitel').textContent = e.titel;
    $('#lbDaten').innerHTML = e.zeilen.map(([k, v]) => `<dt>${L.esc(k)}</dt><dd>${L.esc(v)}</dd>`).join('');
    $('#lbPrompt').textContent = e.prompt || '';
    $('#lbPromptBox').hidden = !e.prompt;
    $('#lbUebernehmen').hidden = !e.uebernehmen;
    $('#lbNochmalGruppe').hidden = !e.nochmal;
    const mehrere = lbListe.length > 1;
    $('#lbZurueck').hidden = !mehrere;
    $('#lbWeiter').hidden = !mehrere;
  }

  function lbBlaettern(schritt) {
    if (lbListe.length < 2) return;
    lbIndex = (lbIndex + schritt + lbListe.length) % lbListe.length;
    lbZeigen();
  }

  // ---------------------------------------------------------------- Aktualisieren

  function renderStatus() {
    const ziel = $('#verbindung');
    const laeuft = S.kurz.filter((k) => k.status === 'laeuft').length;
    const wartet = S.kurz.filter((k) => k.status === 'wartend').length;
    if (S.verbunden === false) { ziel.dataset.zustand = 'aus'; ziel.textContent = 'keine Verbindung'; }
    else if (S.verbunden === null) { ziel.dataset.zustand = 'unbekannt'; ziel.textContent = 'verbinde …'; }
    else if (laeuft || wartet) {
      ziel.dataset.zustand = 'aktiv';
      ziel.textContent = [laeuft ? `${laeuft} läuft` : '', wartet ? `${wartet} wartet` : ''].filter(Boolean).join(' · ');
    } else { ziel.dataset.zustand = 'ok'; ziel.textContent = 'bereit'; }
    const aktiv = laeuft + wartet;
    $('#jobZaehler').hidden = aktiv === 0;
    $('#jobZaehler').textContent = aktiv;
  }

  const braucht = (k) => {
    const d = S.jobs.get(k.id);
    return !d || d.status !== k.status || d.meldung !== k.meldung || bilderFertig(d) !== k.bilder_fertig;
  };

  async function aktualisieren() {
    if (S.polling) return;
    S.polling = true;
    try {
      const vorher = new Map(S.kurz.map((k) => [k.id, k.status]));
      const { jobs } = await api('/jobs');
      S.verbunden = true;
      S.kurz = jobs;
      const ids = new Set(jobs.map((j) => j.id));
      for (const id of [...S.jobs.keys()]) if (!ids.has(id)) S.jobs.delete(id);
      for (const id of [...S.metriken.keys()]) if (!ids.has(id)) { S.metriken.delete(id); S.metrikStand.delete(id); }
      await metrikenLaden(jobs);
      await Promise.all(jobs.filter(braucht).map(async (k) => {
        try { S.jobs.set(k.id, await api(`/jobs/${k.id}`)); } catch { /* beim nächsten Mal */ }
      }));
      // Ein Job ist gerade fertig geworden: Listen kennen neue Bilder.
      if (jobs.some((k) => vorher.has(k.id) && L.istAktiv(vorher.get(k.id)) && !L.istAktiv(k.status))) S.listeVeraltet = true;
      if (S.tab === 'jobs') renderJobs();
      if (S.tab === 'galerie') renderGalerie();
      if (S.tab === 'listen') listeLaden(S.listenArt, { still: true });
    } catch (e) {
      if (e.netz || S.verbunden === null) S.verbunden = false;
    } finally {
      S.polling = false;
      renderStatus();
      planen();
    }
  }

  function planen() {
    clearTimeout(S.timer);
    const aktiv = S.kurz.some((k) => L.istAktiv(k.status));
    const warten = document.hidden ? 15000 : (aktiv ? 1500 : 6000);
    S.timer = setTimeout(aktualisieren, warten);
  }

  // ---------------------------------------------------------------- Verdrahten

  function verdrahten() {
    // Tabs
    for (const b of $$('.tabs [role="tab"]')) b.addEventListener('click', () => tabWechseln(b.dataset.tab));
    $('#tokenKnopf').addEventListener('click', () => tokenAbfragen());

    // Formular
    $('#formular').addEventListener('submit', starten);
    $('#vorschauKnopf').addEventListener('click', vorschau);
    $('#formular').addEventListener('input', formularMerken);
    $('#formular').addEventListener('change', formularMerken);
    $('#prompt').addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); $('#formular').requestSubmit(); }
    });
    for (const r of $$('input[name="seedModus"]')) {
      r.addEventListener('change', () => { $('#seed').disabled = $('input[name="seedModus"]:checked').value !== 'fest'; });
    }
    $('#artWahl').addEventListener('click', (e) => {
      const b = e.target.closest('.segment');
      if (b) { artWaehlen(b.dataset.kind); formularMerken(); }
    });

    // Referenzbild
    const ablage = $('#ablage');
    const datei = $('#datei');
    ablage.addEventListener('click', (e) => { if ($('#ablageVoll').hidden && e.target !== datei) datei.click(); });
    ablage.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && $('#ablageVoll').hidden) { e.preventDefault(); datei.click(); } });
    datei.addEventListener('change', () => dateiHochladen(datei.files[0]));
    ablage.addEventListener('dragover', (e) => { e.preventDefault(); ablage.classList.add('zieht'); });
    ablage.addEventListener('dragleave', () => ablage.classList.remove('zieht'));
    ablage.addEventListener('drop', (e) => {
      e.preventDefault();
      ablage.classList.remove('zieht');
      dateiHochladen(e.dataTransfer.files[0]);
    });
    $('#ablageWeg').addEventListener('click', (e) => { e.stopPropagation(); refEntfernen(); });

    // Jobs
    $('#jobListe').addEventListener('click', (e) => {
      const knopf = e.target.closest('[data-aktion]');
      const karte = e.target.closest('.job');
      if (knopf && karte) jobAktion(knopf.dataset.aktion, karte.dataset.id, knopf);
    });
    $('#aufraeumen').addEventListener('click', beendeteEntfernen);

    // Galerie
    $('#galerieFilter').addEventListener('click', (e) => {
      const chip = e.target.closest('.chip');
      if (chip) { S.galerieFilter = chip.dataset.art; renderGalerie(); }
    });
    $('#galerieRaster').addEventListener('click', (e) => {
      const kachel = e.target.closest('.kachel');
      if (!kachel) return;
      const alle = L.galerie([...S.jobs.values()], S.galerieFilter);
      const i = alle.findIndex((x) => x.job.id === kachel.dataset.job && x.bild.stamm === kachel.dataset.stamm);
      lightboxOeffnen(alle.map(galerieEintrag), Math.max(0, i));
    });

    // Listen
    $('#listenArt').addEventListener('click', (e) => {
      const chip = e.target.closest('.chip');
      if (chip) listeLaden(chip.dataset.art);
    });
    $('#listeSuche').addEventListener('input', renderListe);
    $('#listeRaster').addEventListener('change', (e) => {
      const karte = e.target.closest('.eintrag');
      if (!karte || e.target.type !== 'checkbox') return;
      const schluessel = `${S.listenArt}/${karte.dataset.slug}`;
      if (e.target.checked) S.auswahl.add(schluessel); else S.auswahl.delete(schluessel);
      renderListe();
    });
    $('#listeRaster').addEventListener('click', (e) => {
      const thumb = e.target.closest('[data-aktion="bild"]');
      const karte = e.target.closest('.eintrag');
      if (!thumb || !karte) return;
      const eintrag = S.listen[S.listenArt].eintraege.find((x) => x.slug === karte.dataset.slug);
      listeBildOeffnen(S.listenArt, eintrag, Number(thumb.dataset.index));
    });
    $('#listeAlle').addEventListener('click', () => {
      for (const e of sichtbareEintraege()) S.auswahl.add(`${S.listenArt}/${e.slug}`);
      renderListe();
    });
    $('#listeKeine').addEventListener('click', () => { S.auswahl.clear(); renderListe(); });
    $('#listeAuswahl').addEventListener('click', () => {
      const slugs = [...S.auswahl].filter((k) => k.startsWith(`${S.listenArt}/`)).map((k) => k.slice(S.listenArt.length + 1));
      listeStarten(slugs);
    });
    $('#listeFehlende').addEventListener('click', () => {
      const liste = S.listen[S.listenArt];
      if (!liste) return;
      const alle = ListeOpt().force;
      listeStarten(liste.eintraege.filter((e) => alle || !e.bilder.length).map((e) => e.slug));
    });

    // Lightbox
    $('#lbZu').addEventListener('click', () => lightbox.close());
    $('#lbZurueck').addEventListener('click', () => lbBlaettern(-1));
    $('#lbWeiter').addEventListener('click', () => lbBlaettern(1));
    lightbox.addEventListener('click', (e) => { if (e.target === lightbox) lightbox.close(); });
    lightbox.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowLeft') lbBlaettern(-1);
      if (e.key === 'ArrowRight') lbBlaettern(1);
    });
    $('#lbNochmal').addEventListener('click', async () => {
      const e = lbListe[lbIndex];
      if (!e || !e.nochmal) return;
      const job = await e.nochmal(Number($('#lbAnzahl').value));
      if (job) { lightbox.close(); tabWechseln('jobs'); }
    });
    $('#lbUebernehmen').addEventListener('click', () => { const e = lbListe[lbIndex]; if (e && e.uebernehmen) e.uebernehmen(); });

    document.addEventListener('visibilitychange', () => { if (!document.hidden) aktualisieren(); });
    window.addEventListener('hashchange', () => {
      const name = location.hash.slice(1);
      if (['erstellen', 'jobs', 'galerie', 'listen'].includes(name) && name !== S.tab) tabWechseln(name);
    });
  }

  function artenAufbauen() {
    $('#artWahl').innerHTML = S.arten.map((a) => `<button type="button" class="segment" data-kind="${a.kind}" aria-pressed="false">
      <strong>${L.esc(artName(a.kind))}</strong><small>${L.esc(ART_HINWEIS[a.kind] || '')} · ${a.width}×${a.height}</small></button>`).join('');
  }

  async function start() {
    verdrahten();
    try {
      S.arten = await api('/arten');
    } catch (e) {
      S.verbunden = false;
      renderStatus();
      meldung(`API nicht erreichbar: ${e.message}`, 'fehler');
      planen();
      // Ohne Arten gibt es kein Formular; erneut versuchen, bis die API antwortet.
      const warte = setInterval(async () => {
        try { S.arten = await api('/arten'); clearInterval(warte); start2(); } catch { /* weiter warten */ }
      }, 3000);
      return;
    }
    start2();
  }

  function start2() {
    artenAufbauen();
    artWaehlen(S.kind);
    formularErinnern();
    const hash = location.hash.slice(1);
    tabWechseln(['erstellen', 'jobs', 'galerie', 'listen'].includes(hash) ? hash : 'erstellen');
    aktualisieren();
  }

  start();
})();
