# Kito Files — piano di sviluppo e confronto con Nautilus

Bozza di lavoro aggiornata il **9 ottobre 2026**. Stato Kito rilevato dai sorgenti
del checkout basato su `b770356`, comprese le modifiche locali presenti durante
la ricognizione.
Questo documento pianifica il lavoro: non indica che le funzioni proposte siano già
implementate o approvate nel dettaglio.

## Obiettivo

Raggiungere una copertura funzionale paragonabile a Nautilus nell'uso quotidiano,
mantenendo le scelte di Kito e lasciando spazio a funzioni proprie. Conservare Rust,
GTK4/libadwaita, compatibilità con ambienti diversi, configurazione senza dconf e
interfaccia rapida anche su cartelle grandi.

La parità riguarda i comportamenti utili all'utente. Funzioni fornite da estensioni,
servizi di GNOME o programmi esterni vanno censite separatamente: non diventano
implicitamente dipendenze obbligatorie di Kito.

## Metodo e riferimenti

Il GitLab indicato dall'utente non è stato leggibile tramite il browser di ricerca
(HTTP 406). Sono stati consultati il mirror ufficiale GNOME e la guida ufficiale.
Il riferimento corrente è il ramo `master`, che può cambiare: prima di dichiarare
la parità scegliere una versione/tag di Nautilus e fissare l'inventario a quel riferimento.
La matrice sotto è una prima ricognizione, non un elenco esaustivo certificato.

Fonti usate per il confronto:

- [Repository Nautilus richiesto](https://gitlab.gnome.org/GNOME/nautilus) e
  [mirror ufficiale GNOME](https://github.com/GNOME/nautilus).
- [Guida Files di GNOME](https://help.gnome.org/gnome-help/files.html): navigazione,
  ricerca, rinomina multipla, proprietà, rete, modelli e dispositivi. Alcune pagine
  includono integrazioni dell'ambiente desktop, non funzioni interne di Nautilus.
- [Menu delle azioni](https://github.com/GNOME/nautilus/blob/master/src/resources/ui/nautilus-files-view-context-menus.ui):
  Apri con, nuova scheda/finestra, copia/sposta in, collegamenti, archivi, script,
  recenti, preferiti e azioni sui dispositivi.
- [Preferenze Nautilus](https://github.com/GNOME/nautilus/blob/master/data/org.gnome.nautilus.gschema.xml):
  ordinamento, zoom, colonne, miniature e scelte di ricerca.
- [Operazioni sui file](https://github.com/GNOME/nautilus/blob/master/src/nautilus-file-operations.c):
  avanzamento, cancellazione, conflitti, unione delle cartelle e supporto undo.
- [Vista e caricamento](https://github.com/GNOME/nautilus/blob/master/src/nautilus-files-view.c):
  indicatore flottante ritardato di 200 ms;
  [enumerazione asincrona](https://github.com/GNOME/nautilus/blob/master/src/nautilus-directory-async.c).
- [Anteprima](https://help.gnome.org/gnome-help/files-preview.html),
  [ricerca](https://help.gnome.org/gnome-help/files-search.html) e
  [modelli](https://help.gnome.org/gnome-help/files-templates.html): riferimenti per
  distinguere comportamento e servizi necessari.

I file Nautilus consultati riportano GPL-2.0-or-later; Kito è MIT. Per questo piano
si studiano i comportamenti e si progetta un'implementazione propria. Qualsiasi
riuso di codice o asset richiede prima una verifica della compatibilità delle licenze.

## Stato attuale di Kito

“Presente” significa trovato nei sorgenti, non validato in ogni ambiente.
“Parziale” significa che esiste una base ma manca parte del comportamento.

| Area | Stato | Evidenza / limite attuale |
|---|---|---|
| Navigazione, breadcrumb, schede e cronologia | Presente | `main.rs`, `tabs.rs`; cronologia confermata dopo caricamento riuscito |
| Percorso modificabile e autocompletamento locale | Presente | `path_completion.rs`; da collaudare insieme ai nuovi caricamenti |
| Icone, Compatta, Dettagli | Presente | `file_list.rs`; dimensioni e colonne ancora poco configurabili |
| Caricamento asincrono e inserimenti in blocco | Presente | `tabs.rs`, `file_list.rs`; pagina Caricamento immediata ancora presente |
| Copia/taglio/incolla, rinomina, cartelle | Presente | `ops.rs`, `kito-core`; errori operativi ancora poco dettagliati |
| Operazioni su gruppi di file | Mancante | Le viste usano `SingleSelection` |
| Cestino, ripristino e cancellazione | Presente | Protezioni symlink e destinazioni di copia; verifiche locali/remote da estendere |
| Conflitti e spostamenti tra filesystem | Parziale | Suffissi automatici; niente dialogo unisci/sostituisci; `move_to()` usa GIO senza fallback esplicito per cartelle |
| Avanzamento, interruzione e undo delle operazioni | Mancante | Esiti via toast; nessun gestore generale delle operazioni |
| Aggiornamento live della cartella aperta | Mancante | Monitor presenti per segnalibri, non un monitor generale dei contenuti della vista |
| Segnalibri e cartelle XDG | Presente | Segnalibri GTK condivisi; gestione e riordino da ampliare |
| Dispositivi e rete | Parziale | Mount e voce network; la sidebar offre espulsione/smontaggio quando supportati da GIO; restano autenticazione e connessioni |
| Proprietà | Parziale | Metadati di base; mancano modifica permessi e gestione completa delle selezioni |
| Menu contestuali | Parziale | Popover manuali; sottomenu con chiusure a cascata, proprietà dello sfondo dipendenti dalla selezione |
| Terminale e shell amministratore | Presente | Scelta emulatori; modifiche locali in corso, da preservare e verificare |
| Preferenze, controlli finestra, inglese/italiano | Presente | Configurazione XDG e cataloghi Fluent |
| Ricerca, miniature, archivi, Apri con | Mancante | Non individuate implementazioni nei moduli attuali |

Moduli di riferimento: [UI](../crates/kito-gtk/src/main.rs),
[schede](../crates/kito-gtk/src/tabs.rs), [operazioni](../crates/kito-gtk/src/ops.rs),
[backend](../crates/kito-core/src/lib.rs), [menu](../crates/kito-gtk/src/context_menu.rs),
[preferenze](../crates/kito-gtk/src/preferences/model.rs).

## Cosa mantenere di Kito

Queste sono scelte da preservare; non si sostiene che siano tutte esclusive di Kito.

- Vista Compatta oltre a Icone e Dettagli.
- Scelta esplicita dell'emulatore e shell amministratore nel terminale.
- Lingua selezionabile nell'app e fallback inglese.
- Controlli minimizza/massimizza/chiudi personalizzabili.
- Configurazione semplice XDG, senza richiedere dconf, shell GNOME o indicizzazione.
- Segnalibri freedesktop condivisi, tema e icone del sistema.
- Operazioni e test del backend separati dai widget.
- Percorso modificabile con autocompletamento e obiettivo di reattività misurabile.

## Inventario da raggiungere

Le capacità Nautilus in questa tabella derivano dai riferimenti sopra. Fasi e scelte
Kito sono proposte progettuali. Difficoltà indicative, senza stime temporali.

| ID | Funzione | Kito oggi | Fase proposta | Difficoltà |
|---|---|---|---|---|
| F01 | Selezione multipla, intervalli, seleziona tutto/inverti | Mancante | 1 | Media |
| F02 | Trascinamento tra cartelle, schede, sidebar e altre app | Mancante | 1 | Alta |
| F03 | Apri in nuova scheda/finestra; clic centrale e scorciatoie | Parziale | 1 | Bassa/media |
| F04 | Aggiornamento live dei contenuti con monitor | Mancante | 1 | Media |
| F05 | Ordinamento per nome, dimensione, tipo e data, inverso | Parziale: nome fisso | 1 | Media |
| F06 | Zoom delle icone e colonne selezionabili/ordinabili | Mancante | 1 | Media |
| F07 | Scorciatoie complete, focus e accessibilità | Parziale | Trasversale | Media |
| F08 | Centro operazioni, avanzamento e interruzione | Mancante | 2 | Alta |
| F09 | Sostituisci, salta, conserva entrambi, unisci cartelle | Parziale: suffissi | 2 | Alta |
| F10 | Spostamento affidabile tra filesystem | Parziale | 2 | Alta |
| F11 | Annulla/ripeti per operazioni reversibili | Mancante | 2 | Alta |
| F12 | Copia in / Sposta in / Incolla nella cartella selezionata | Mancante | 2 | Media |
| F13 | Creazione di collegamenti simbolici | Mancante | 2 | Media |
| F14 | Ricerca per nome, ricorsiva, filtri tipo/data | Mancante | 3 | Alta |
| F15 | Ricerca nel contenuto con backend opzionale | Mancante | 5 | Alta |
| F16 | Miniature e cache; limiti per locale/remoto | Mancante | 3 | Alta |
| F17 | Anteprima rapida di file | Mancante | 3 | Alta |
| F18 | Apri con, elenco applicazioni e predefinita MIME | Mancante | 3 | Media |
| F19 | Recenti e rimozione dalla cronologia | Mancante | 3 | Media |
| F20 | Preferiti separati dai segnalibri delle cartelle | Mancante | 3 | Media/alta |
| F21 | Rinomina multipla con anteprima | Mancante | 4 | Alta |
| F22 | Modelli dalla directory XDG Templates | Mancante | 4 | Media |
| F23 | Compressione ed estrazione integrate | Mancante | 4 | Alta |
| F24 | Proprietà aggregate, spazio libero, permessi | Parziale | 4 | Alta |
| F25 | Connessione server, autenticazione e connessioni salvate | Parziale | 4 | Alta |
| F26 | Smonta/espelli, volumi cifrati e media rimovibili | Parziale | 4 | Alta |
| F27 | Script utente e sezioni del menu estendibili | Mancante | 5 | Alta |
| F28 | Azioni di sistema: invio email, sfondo, servizi esterni | Mancante | 5, opzionale | Variabile |
| F29 | Metadati utili: emblemi symlink, accesso negato, informazioni cartelle | Parziale | 3–4 | Media |

Non trasformare questa tabella in una promessa “tutto Nautilus”: completarla contro
il tag scelto, includendo menu, scorciatoie, preferenze, protocolli e integrazioni.

## Fase 0 — completare ciò che è già iniziato

- [x] **B01 — Menu sullo sfondo — implementazione e test automatici completati.**
  Menu compatto con righe a icone come il menu dei file, senza contenitore a
  scorrimento; Nuovo file usa un popover separato con icone per i tipi di file;
  contesto di tab/directory catturato, menu del cestino dedicato e proprietà
  indipendenti dalla selezione. Modelli e disponibilità contestuale hanno test
  automatici.
  - [x] **Verifica grafica su Wayland completata** (conferma utente): sottomenu,
    Escape, clic esterno, tastiera, selezione, proprietà, aperture ripetute e
    conferme del menu del cestino.
- [x] **B02 — Caricamento discreto — implementazione e test di stato completati.**
  Worker GIO cancellabile, indicatore differito di 200 ms, modello costruito a
  blocchi e commit atomico di contenuto/percorso/cronologia. Test automatici
  coprono cancellazione, generazioni e risultati fuori ordine. L’ordinamento
  sincrono nel worker non è interrompibile a metà: la cancellazione viene
  recepita prima o dopo il sort. I log separano enumerazione, sort e
  build/applicazione del modello.
  - [x] **Verifica grafica su Wayland completata** (conferma utente): soglia e
    rendering dell’indicatore, interazione durante il caricamento e fluidità.
    Non sono riportate nuove misure prestazionali numeriche.
- [x] **B03 — Errori operativi — implementazione e test automatici completati.**
  Risultati strutturati per elemento, riepilogo localizzato con dettagli
  espandibili, ritentativi dei soli elementi non riusciti e controlli di
  generazione degli appunti. I test automatici coprono backend file, appunti,
  cancellazione dei symlink, protezione dalle copie ricorsive, ripristino e
  candidati di retry.
  - [x] **Verifica grafica su Wayland completata** (conferma utente): dialoghi
    con errori parziali e flusso appunti da una sessione desktop.
- [x] **B04 — Matrice di verifica — compilata e aggiornata.** La tabella in
  `docs/VERIFICATION.md` riporta i controlli automatici e le verifiche grafiche
  Wayland riferite dall’utente.
  - [ ] **Copertura X11 residua:** eseguire le righe grafiche in una sessione
    X11 separata; Wayland è l’unico backend provato finora.

Uscita dalla fase: navigazione rapida e menu affidabili, nessuna regressione su
cancellazione, copia in sé stessi, ripristino e appunti. Le modifiche locali in corso
non vanno sovrascritte. I prompt già discussi sono requisiti, non prove di completamento.

## Fase 1 — uso quotidiano

- [x] **F01 — implementazione e verifiche automatiche completate.** Selezione
  multipla nelle tre viste, Ctrl/Shift-click, Ctrl+A, inverti/deseleziona,
  selezione URI persistente e rimozione delle voci mancanti. Le azioni di gruppo
  catturano URI e destinazione; Rinomina/Proprietà restano azioni singole.
  - [x] Test headless su riordino, rimozione di selezionati, snapshot trasferimento
    e retry dei soli fallimenti.
  - [ ] Verifica grafica di click, intervalli, focus, menu e operazioni di gruppo.
- [x] **F03/F07 — implementazione e verifiche automatiche completate.** Nuova
  scheda/finestra per cartelle selezionate, clic centrale in background, scorciatoie,
  cronologia in memoria di 20 schede chiuse con stato di navigazione e vista.
  Questa riapertura durante la sessione è distinta dalle sessioni salvate N02.
  - [x] Test headless su limite di 20, round-trip dello snapshot e risultati obsoleti dopo chiusura.
  - [ ] Verifica grafica di apertura/chiusura, scorciatoie, accessibilità e focus.
- [x] **UI01 — Icone coerenti. Implementazione completata; verifica grafica
  residua (F07).** Sidebar, toolbar, pulsanti delle
  azioni e menu usano icone simboliche del tema di sistema, con priorità alle
  varianti `-symbolic`; file e cartelle nelle viste Icone/Compatta/Dettagli mantengono
  le icone normali GIO del tema, senza forzarle monocromatiche. Riutilizzare gli
  helper esistenti distinguendo icone dei controlli e icone dei contenuti.
  Sidebar: nomi simbolici per Home, cartelle XDG, segnalibri, rete e cestino
  vuoto/pieno; dispositivi da icone simboliche GIO quando disponibili. Fallback
  semantici simbolici prima delle varianti normali, senza icone mancanti o
  dipendenze obbligatorie da Adwaita/Papirus/Breeze. Lasciare a GTK colori e stati;
  dimensioni e allineamento coerenti, tema chiaro/scuro e contrasto verificati.
  Nessuna nuova voce Recenti/Preferiti soltanto per aggiungere la relativa icona.
  Preservare azioni, tooltip, accessibilità e aggiornamento del cestino. Verificare
  cambio tema, icone mancanti e resa con i temi effettivamente disponibili;
  lo stile segue il sistema, non promette una copia identica di Nautilus.
  - [x] Implementazione: helper per icone dei controlli, fallback simbolici,
    icone GIO dei dispositivi e icone normali dei file preservate nelle tre viste;
    controlli automatici completati.
  - [ ] Verifica grafica: sidebar, toolbar e menu; cestino vuoto/pieno isolato;
    temi chiaro/scuro, stati dei controlli, fallback visibili e cambio tema;
    resa delle icone dei file nelle tre viste. Sessione grafica non disponibile
    in questa verifica; vedere `docs/VERIFICATION.md`.
- [x] **UI02 — Scorciatoie visibili nei menu. Implementazione completata;
  verifica grafica residua (F07).** Nei menu dell'app,
  contestuali di file/sfondo/cestino, sidebar e sottomenu mostrare la scorciatoia
  realmente disponibile per l'azione. Posizionarla dopo il nome dell'azione, in
  una colonna allineata **al bordo destro del menu**, conservando icone e
  struttura dei menu. Derivare le
  indicazioni dalla stessa fonte delle combinazioni effettivamente registrate,
  comprese quelle gestite da controller; nessuna mappa duplicata o scorciatoia
  inventata. Azioni contestuali mostrano la combinazione solo se equivalente
  all'azione da tastiera nel contesto corrente; nessuna indicazione per azioni
  prive di scorciatoia. Usare etichette leggibili GTK, stati disabilitati coerenti,
  allineamento e contrasto adatti ai temi chiaro/scuro, senza aggiungere larghezze
  o scroll inutili. Preservare focus, navigazione da tastiera, conferme e semantica
  delle azioni. Verificare menu riaperti, cambio lingua, scorciatoie alternative
  e correttezza della corrispondenza tra indicazione e comportamento; coordinare
  con UI01 e con le nuove azioni della Fase 1.
  - [x] Implementazione e test automatici: fonte condivisa per acceleratori
    globali e locali alla vista; colonne con formattazione GTK; verifica delle
    equivalenze contestuali e delle alternative.
  - [ ] Verifica grafica: corrispondenza tasti/azioni, tutti i menu e sottomenu,
    righe disabilitate, cambio lingua, allineamento/contrasto, focus nei campi e
    conferme. Sessione grafica non disponibile in questa verifica; vedere
    `docs/VERIFICATION.md`.
- [ ] **UI03 — Composizione coordinata di intestazione e sidebar.** Ispirandosi
  al riferimento visivo fornito dall'utente, organizzare la finestra in due aree
  verticali coerenti dall'intestazione fino al contenuto: a sinistra i controlli
  e la sidebar, a destra navigazione/percorso e area dei file. Il confine deve
  restare allineato quando si ridimensiona il divisore dei pannelli; preservare
  controlli finestra, percorso modificabile, navigazione, schede e selettore vista,
  evitando una seconda riga di toolbar o azioni non funzionanti. Riordinare la
  sidebar in gruppi visivamente chiari: Home, Preferiti, Rete e Cestino; cartelle
  XDG; dispositivi montati, con separatori discreti e azioni di volume accessibili.
  Mantenere bookmark e dispositivi dinamici, selezione coerente con scheda e
  destinazione, traduzioni, accessibilità e temi GTK chiari/scuri; nessuna
  dipendenza da GNOME né colori rigidi copiati dallo screenshot. Verificare
  ridimensionamento, finestre strette, liste lunghe, più schede e sessioni Wayland
  e X11 quando disponibili. Lo screenshot è un riferimento di layout, non una
  richiesta di cambiare framework o copiare l'interfaccia pixel per pixel.
  - [x] Implementazione: un unico `GtkPaned` contiene entrambe le colonne dalla
    barra superiore al contenuto; header sidebar con icona ricerca allineata
    alla colonna delle icone, titolo centrato e menu a destra. La ricerca resta
    un'immagine informativa finché F14 non fornisce una funzione reale. Sidebar raggruppata, Preferiti
    espandibile per i bookmark di cartelle, etichette URI leggibili, deduplicazione
    XDG, selezione per antenato GIO, azioni volume accessibili quando supportate e
    griglia Icone più ampia (quattro colonne a zoom 100% nella finestra di riferimento).
  - [x] `cargo check`, build workspace e test workspace (escluso il test che usa
    il cestino della sessione) superati.
  - [ ] Verifica grafica interattiva residua: trascinamento del divisore, finestre
    strette, cambio scheda/lingua/tema, dinamica di preferiti e volumi; Wayland
    provato all'avvio, non ispezionato a schermo. X11 controllato su XWayland,
    non in una sessione X11 separata.
- [x] **F05/F06 — implementazione e verifiche automatiche completate.** Ordinamento
  naturale per nome/dimensione/tipo/data, per-scheda; zoom e colonne globali
  persistenti; selezione URI e ancora di scorrimento preservate. Metadati via GIO,
  senza dimensioni ricorsive o I/O nel comparatore.
  - [x] Test su ordine naturale, sequenze numeriche lunghe, parità, metadati mancanti,
    preferenze e confronti per tutti i campi/direzioni.
  - [ ] Verifica grafica di intestazioni, zoom, scorciatoie e conservazione dello scroll.
- [x] **F04 — implementazione e verifiche automatiche completate.** Monitor GIO per
  scheda, batch coalesciuti e limitati, aggiornamenti mirati e riconciliazioni in
  worker; fallback esplicito quando il backend rifiuta il monitor. Chiusura scheda
  rilascia monitor e lavoro pendente.
  - [x] Test su raffiche, overflow, eventi conflittuali e risultati obsoleti.
  - [ ] Verifica grafica di creazione, modifica, rename, rimozione e navigazione rapida.
- [ ] **F02 — parziale.** Drop interno su cartelle, sfondo, schede e sidebar; sorgente
  GDK offre `FileList` e `text/uri-list`, destinazioni negoziano soltanto Copy/Move.
  Trasferimenti esterni su cartelle usano il protocollo Move solo dopo il successo;
  Copy su schede e Move interno sulla barra schede sono collegati. Move esterno sulla
  barra schede viene rifiutato perché il callback libadwaita è sincrono e non può
  confermare un trasferimento asincrono. Link, hover-apertura e autoscroll restano
  fuori dal completamento.
  - [x] Test headless su negoziazione, MIME URI list, retry, protezione symlink e
    destinazioni sé-stesse/discendenti nel backend.
  - [ ] Verifica grafica di drop interni/esterni su tutte le destinazioni e con altre app.

Uscita **non raggiunta**. Mancano verifiche grafiche e scambio reale con altre app;
il Move esterno sulla barra schede resta rifiutato. La misura release su 50.000
elementi ha mostrato variabilità fra processi e, nell'ultima serie accoppiata,
un totale dell'8.2% più alto; non si dichiara assenza di regressioni prestazionali.
UI01 e UI02 restano inoltre attività separate, da coordinare con i controlli della
fase. Ripetere le misure su un host stabile e completare le prove Wayland/X11 prima
di chiudere la fase.

## Fase 2 — motore delle operazioni

- [ ] **F08:** introdurre job con ID, sorgenti/destinazione catturate, stato, avanzamento,
  cancellazione cooperativa e risultato per elemento; concorrenza limitata.
  UI centrale con dettagli; non inventare percentuali quando il totale è sconosciuto.
- [ ] **F09:** dialoghi conflitto e scelta applicabile ai successivi; nomi originali
  preservati; unione distinta dalla sostituzione; verifica scrivibilità solo indicativa.
- [ ] **F10:** copia seguita da cancellazione per gli spostamenti che la richiedono;
  cancellare la sorgente soltanto dopo successo verificato. Definire trattamento di
  symlink, permessi, metadati, interruzioni e file modificati durante il trasferimento.
- [ ] **F12/F13:** destinazioni esplicite e creazione collegamenti senza ambiguità
  tra il contesto dello sfondo e la selezione.
  Per F12 aggiungere destinazioni rapide configurabili e ultime destinazioni usate,
  condividendo lo stesso motore di Copia in / Sposta in; gestire percorsi rimossi
  senza trasferimenti automatici e permettere di cancellare la cronologia.
- [ ] **F11:** undo/redo sopra il registro dei job; copertura iniziale limitata a
  rinomina, spostamento, creazione e cestino. Gestire conflitti e modifiche esterne;
  nessuna promessa di annullare cancellazioni permanenti o svuotamento del cestino.

Dipendenze: F01 prima delle azioni di gruppo; F08/F09 prima di F11. Non aggiungere
una pila undo soltanto nella UI: servono esiti e identità effettive del backend.

## Fase 3 — trovare e riconoscere i file

- [ ] **F14:** ricerca per nome nella cartella e sottocartelle senza servizio obbligatorio;
  debounce, interruzione, limiti e risultati progressivi. Filtri tipo/data, apertura
  della posizione originale e azioni sui risultati con contesto esplicito.
- [ ] **F16:** miniature asincrone con cache e priorità agli elementi visibili;
  limiti di dimensione/concorrenza, invalidazione e opzione locale/mai/remoto.
- [ ] **F17:** iniziare da immagini e testo o integrare un visualizzatore opzionale;
  spazio apre l'anteprima, Escape la chiude. PDF/video solo con dipendenze definite.
  La guida GNOME descrive l'anteprima come integrazione che può richiedere software aggiuntivo.
- [ ] **F18:** applicazioni GIO associate al MIME, scelta una tantum o predefinita;
  corretta gestione di selezioni miste, file remoti e applicazioni non disponibili.
- [ ] **F19:** fonte interoperabile dei recenti da scegliere; rimuovere dalla cronologia
  non elimina il file. Preferenze per privacy e disponibilità fuori da GNOME.
- [ ] **F20:** distinguere cartelle appuntate e file preferiti; storage portabile,
  gestione di rinomina/spostamento e nessun indicizzatore obbligatorio.
- [ ] **F29:** emblemi e metadati differiti senza bloccare il caricamento principale.
  Per i symlink mostrare la destinazione, consentire di raggiungerla e distinguere
  collegamenti interrotti o non accessibili. L'apertura della destinazione non
  modifica il collegamento; copie e cancellazioni conservano la propria semantica.

Dipendenze: F04 favorisce cache corrette; F01/F12 consentono azioni affidabili sui risultati.

## Fase 4 — documenti, archivi e dispositivi

- [ ] **F22:** copiare veri modelli da XDG Templates, incluse eventuali sottocartelle;
  collisioni, directory vuota e file non leggibili. Sostituire i falsi documenti Office
  vuoti con modelli validi, senza generare formati documentali da zero.
- [ ] **F21:** rinomina multipla con sostituzioni e numerazione; anteprima completa,
  collisioni, nomi Unicode e cicli A→B/B→A. Una singola operazione reversibile.
- [ ] **F23:** backend archivi da scegliere, formati dichiarati e job cancellabili;
  estrazione non deve scrivere fuori dalla destinazione attraverso nomi o symlink.
  Non promettere tutti i formati presenti sul computer.
- [ ] **F24:** dimensione aggregata calcolata in background, tipo, date, MIME,
  permessi Unix e opzione eseguibile; capability dei backend remoti. Modifiche
  ricorsive separate e chiaramente confermate.
  Prima di un trasferimento confrontare dimensione stimata e spazio libero noto
  nella destinazione, senza rallentare la navigazione. Segnalare insufficienza nota
  e distinguere dati sconosciuti; non promettere il successo, perché spazio, quote
  e costi effettivi possono cambiare durante l'operazione.
- [ ] **F25:** form connessione server, mount/auth GIO, errori e disconnessione;
  testare SMB, SFTP e WebDAV quando i backend sono installati. Credenziali mai
  salvate in chiaro nel file delle preferenze.
- [ ] **F26 — residuo:** autenticazione di mount, volumi cifrati e gestione dei
  dispositivi rimossi durante un job. La sidebar espone già smontaggio/espulsione
  tramite GIO quando supportati (UI03); nessuna operazione è presentata come
  riuscita se fallisce.

Archivi, autenticazione, permessi e spostamenti richiedono prove dedicate prima di
considerare la fase completa; un menu visibile non equivale a una funzione pronta.

## Fase 5 — integrazioni e personalizzazione

- [ ] **F15:** ricerca nel contenuto opzionale, con backend selezionabile; conservare
  ricerca per nome senza indicizzatore. Definire costo, privacy e limiti dei formati.
- [ ] **F27:** script utente con selezione passata senza interpolazioni shell,
  directory di lavoro esplicita e timeout/cancellazione; modello di estensioni da
  progettare separatamente. Non promettere compatibilità ABI con plugin Nautilus.
- [ ] **F28:** invio email, impostazione sfondo e altri servizi esposti solo se
  disponibili; adattatori opzionali, senza obbligare Kito a dipendere dal desktop GNOME.
  Includere condivisione attraverso servizi installati, con scelta esplicita di
  servizio e file; non vincolare l'app a un fornitore e non avviare invii automatici.
- [ ] **Audit finale:** confrontare ogni voce con il tag Nautilus scelto; distinguere
  equivalenza, differenza intenzionale, integrazione opzionale e funzionalità mancante.

## Funzioni proprie — nice to have

Extra inseriti nel piano su richiesta dell'utente. Restano da progettare: l'inclusione
nella roadmap non autorizza ancora l'implementazione né fissa tempi di consegna.
Le priorità sono proposte; completare prima le basi da cui ciascun extra dipende.

### Prima priorità proposta

- [ ] **N01 — Copia percorso.** Copiare nome, percorso locale completo o URI con
  azioni distinte, anche per selezioni multiple. Non confondere il testo copiato
  con gli appunti di copia/taglio dei file. Dopo B01; per gruppi di file, dopo F01.
- [ ] **N02 — Sessioni salvate.** Ripristino opzionale delle schede e gruppi di lavoro
  nominati, come Lavoro, Foto o Server. Gestire percorsi rimossi e posizioni remote
  senza bloccare l'avvio; non salvare credenziali. Dopo B02 e F03, usando la
  configurazione XDG esistente.
- [ ] **N03 — Due pannelli opzionali.** Sorgente e destinazione affiancate, con
  navigazione e selezione indipendenti, pannello attivo riconoscibile e ritorno alla
  vista singola. Dopo F01, F02 e il contesto delle azioni; trasferimenti attraverso
  lo stesso motore di operazioni F08, senza una seconda implementazione.

### Produttività e controllo

- [ ] **N04 — Azioni personalizzate.** Comandi configurabili per selezione o cartella,
  con argomenti strutturati e contesto esplicito, senza interpolazioni shell
  implicite. Dopo B01 e F01; progettare insieme a F27, distinguendo azioni
  configurate e script utente per evitare due sistemi incompatibili.
- [ ] **N05 — Palette comandi.** Cercare azioni e preferenze da tastiera; mostrare
  scorciatoie e disponibilità nel contesto corrente. Riutilizzare le azioni GIO
  esistenti, senza duplicarne la logica. Dopo B01 e F07.
- [ ] **N06 — Checksum su richiesta.** Calcolare SHA-256 e confrontarlo con un valore
  atteso o un altro file. Lettura in background, avanzamento e interruzione; segnalare
  se il file cambia durante il calcolo. Dopo F08; niente hashing automatico all'apertura.
- [ ] **N07 — Cronologia delle operazioni.** Consultare sorgenti, destinazioni, esiti
  ed errori e raggiungere la destinazione. Dopo B03 e F08; collegare undo/redo a F11
  quando disponibile. Definire persistenza, durata e cancellazione della cronologia,
  separandola dai file recenti F19.
- [ ] **N08 — Spazio occupato.** Scansione delle sottocartelle e vista delle dimensioni
  per individuare quelle più pesanti. Dopo F24 e il servizio dei job; distinguere
  dimensione logica e spazio allocato quando disponibile, gestire hard link, symlink,
  errori di accesso e annullamento. Nessuna scansione profonda automatica nella vista.

### Comodità quotidiane

- [ ] **N12 — Filtro rapido nella cartella.** Filtrare i nomi della directory corrente
  senza cercare nelle sottocartelle e senza avviare F14. Indicatore del filtro,
  ripristino della lista completa e stato distinto per nessuna corrispondenza.
  Non intercettare la digitazione nella barra percorso o nei dialoghi; definire
  cosa accade agli elementi selezionati nascosti dal filtro. Dopo F01 e F05/F06.
- [ ] **N13 — Incolla immagini dagli appunti.** Salvare un'immagine come PNG nella
  cartella scelta, chiedendo nome e gestendo collisioni. Distinguere contenuti
  immagine da file/URI e testo, senza alterare la semantica del taglio. Lettura
  asincrona, limiti di memoria e scarto dei risultati obsoleti; dopo B03 e F08.
- [ ] **N14 — Crea cartella dalla selezione.** Creare una cartella nominata e spostarvi
  gli elementi selezionati usando il motore comune. Dopo F01, F08 e F09; riportare
  successi parziali senza perdere file né ripetere spostamenti già riusciti.
  Integrare l'annullamento con F11 quando disponibile e definire il destino della
  cartella creata se nessun elemento viene trasferito.
- [ ] **N15 — Rinomina conservando l'estensione.** Nel dialogo di un file selezionare
  inizialmente il nome senza estensione, lasciando possibile modificarla con
  un'indicazione chiara del cambio. Definire dotfile, nomi senza estensione e
  suffissi composti come `.tar.gz`; le cartelle non hanno questa distinzione.
  Dopo B03; coordinare con la rinomina multipla F21 senza duplicarne la logica.
- [ ] **N16 — Preferenze per cartella.** Ricordare vista, ordinamento e zoom per URI,
  con opzione per ripristinare i valori globali. Dopo F05/F06; definire precedenza
  tra default, stato della scheda e scelta della cartella, limiti dello storage e
  comportamento con rinomina, symlink e posizioni remote. Nessuna credenziale negli URI
  salvati e nessuna scrittura di configurazione dentro le cartelle dell'utente.
- [ ] **N17 — Sidebar personalizzabile: mostra/nascondi voci.** Nelle Preferenze
  consentire di nascondere o ripristinare qualsiasi singola voce della sidebar
  sinistra, inclusi Home, cartelle XDG, cestino, rete, segnalibri e dispositivi;
  includere Recenti/Preferiti quando saranno implementati. Nascondere l'intera
  riga (icona e testo), senza eliminare file o segnalibri, smontare dispositivi
  o disabilitare la funzione corrispondente. Scelte globali persistenti in
  `settings.conf`, applicate subito alle finestre aperte e conservate al riavvio;
  default uguale alla sidebar attuale e pulsante per ripristinare tutte le voci.
  Usare identificativi stabili, non etichette tradotte o indici; definire la
  persistenza delle voci dinamiche e il comportamento dei nuovi dispositivi.
  Nascondere intestazioni/separatori delle sezioni vuote, consentire anche una
  sidebar senza voci e mantenere le Preferenze raggiungibili dal menu dell'app.
  Nascondere una destinazione non cambia la cartella già aperta. Verificare
  persistenza, cambio lingua, finestre multiple e rimozione/ricomparsa di volumi
  e segnalibri. Coordinare con UI01, senza introdurre dipendenze dalle fasi avanzate.

### Analisi e trasferimenti avanzati

- [ ] **N09 — Confronto cartelle.** Evidenziare elementi mancanti e differenti usando
  prima nomi/metadati; confronto del contenuto opzionale, senza dichiarare identici
  file verificati solo per dimensione/data. Dopo F08 e N06. Il confronto iniziale
  non modifica i file; i trasferimenti scelti usano il motore e i conflitti F09.
- [ ] **N10 — Ricerca duplicati.** Scansione progressiva per dimensione e contenuto,
  con risultati verificabili e tutte le posizioni visibili. Dopo N06 e F08; gestire
  hard link e file modificati durante la scansione. Nessuna cancellazione automatica:
  eventuali rimozioni passano dalle normali azioni e conferme.
- [ ] **N11 — Sincronizzazione con anteprima, eventuale.** Progetto successivo e
  separato da N09: definire direzione, conflitti, cancellazioni e gestione delle
  modifiche concorrenti. Richiede N09, F09 e F10, con piano visibile prima
  dell'esecuzione e scelta esplicita dell'utente. Non è implicita nel confronto.

Ordine iniziale proposto: **N01 → N02 → N03**, rispettando le dipendenze sopra.
Gli altri extra possono essere inseriti dopo il relativo lavoro di base, senza
ritardare stabilità, selezione multipla, ricerca e gestione delle operazioni.
Tra le comodità quotidiane, privilegiare **N12, N13 e N14** una volta completate
le rispettive dipendenze. Riapertura schede, destinazioni rapide, gestione symlink,
spazio libero e condivisione sono integrati in F03/F07, F12, F29, F24 e F28:
non costituiscono ticket duplicati.

## Architettura proposta

1. **Modello vista:** elementi identificati da URI, selezione multipla, ordinamento,
   filtri e generazione del caricamento; indipendente dal riciclo dei widget.
2. **Contesto azioni:** scheda, directory, selezione e capacità catturate al momento
   dell'attivazione. Sfondo, risultati ricerca e selezione hanno contesti distinti.
3. **Servizio operazioni:** job del backend e flusso di eventi limitato; base comune
   per paste, drag-and-drop, archivi e undo. Mai widget GTK nei worker.
4. **Servizi opzionali:** thumbnail, anteprima, ricerca contenuto, recenti e desktop;
   funzionalità esplicite quando un backend manca.
5. **Preferenze:** continuare con configurazione XDG e valori migrabili; definire
   ambito globale/per finestra/per cartella prima di aggiungere persistenza.

Questa è una direzione di progetto, non un ordine di riscrivere l'app: estrarre i
componenti quando una funzione concreta lo richiede, preservando i test esistenti.

## Criteri comuni di completamento

Ogni ticket deve specificare comportamento, dipendenze, rischi, prove e limiti.
Stati consentiti: da progettare → pronto → in sviluppo → da verificare → completato.

- Comportamento verificato con casi positivi, errore, annullamento e concorrenza.
- Nessuna operazione su file personali nei test; fixture temporanee e backend simulati.
- Inglese e italiano, tastiera, focus, accessibilità e tema chiaro/scuro.
- Schede e finestre multiple; chiusura durante attività; risultati obsoleti ignorati.
- URI remoti, Unicode, spazi, nomi non UTF-8 dove il filesystem li ammette e symlink.
- UI reattiva, memoria e concorrenza limitate, misure su cartelle 100/10.000/50.000;
  distinguere primo caricamento da cache calda e tempo totale da fluidità percepita.
- Test, compilazione, formattazione e lint pertinenti; verifiche grafiche dichiarate.
- Documentazione dettagliata in USAGE/DEVELOPMENT; README pubblico resta breve e
  l'alert iniziale resta invariato. Aggiornare la matrice solo dopo implementazione.

## Decisioni aperte prima dei ticket avanzati

- Versione/tag Nautilus da usare come traguardo di parità.
- Backend di archivi, miniature, anteprima e ricerca contenuto: dipendenze e fallback.
- Politica conflitti di default e confini dell'undo.
- Fonte recenti/preferiti e persistenza delle scelte per cartella.
- Priorità e dettagli delle funzioni N01–N17; in particolare decidere se promuovere
  la sincronizzazione N11 a requisito, senza rinviare la stabilità di base.

## Primo lotto consigliato

**B01 → B02 → F01 → F03/F07 → F05/F06 → F04 → F02.**

Menu e caricamento migliorano il comportamento già discusso; selezione multipla e
ordinamento eliminano limiti quotidiani. Il gestore dei job della fase 2 viene prima
di undo, archivi e operazioni complesse. Questo ordine è proposto e può essere rivisto.
