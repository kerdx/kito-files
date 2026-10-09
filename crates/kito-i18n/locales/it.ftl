## Menu contestuale
menu-open = Apri
menu-open-new-tab = Apri in una nuova scheda
menu-open-new-window = Apri in una nuova finestra
open-folders-only = Seleziona cartelle da aprire in nuove schede o in una nuova finestra
menu-pin = Aggiungi ai luoghi
menu-cut = Taglia
menu-copy = Copia
menu-paste = Incolla
menu-rename = Rinomina…
menu-trash = Sposta nel cestino
menu-delete = Elimina definitivamente…
menu-properties = Proprietà
menu-select-all = Seleziona tutto
menu-invert-selection = Inverti selezione
menu-deselect-all = Deseleziona tutto
menu-restore = Ripristina
menu-create = Crea
menu-new-folder = Nuova cartella
menu-new-text-file = Nuovo file di testo
menu-new-empty-file = Nuovo file vuoto
menu-new-word-doc = Documento Word
menu-new-spreadsheet = Foglio di calcolo
menu-new-html = Pagina HTML
menu-open-terminal = Apri terminale
menu-open-terminal-root = Apri terminale come root
menu-empty-trash = Svuota cestino…
bg-new-folder = Nuova cartella…
bg-new-file = Nuovo file…
bg-folder-properties = Proprietà della cartella

## Barra superiore
nav-back = Indietro
nav-forward = Avanti
nav-up = Livello superiore
nav-new-tab = Nuova scheda
menu-application = Menu dell’applicazione
view-selector = Opzioni vista
view-current = Vista attuale: { $view }
view-icons = Icone
view-compact = Compatta
view-details = Dettagli
view-hidden = Mostra file nascosti
view-columns = Colonne dei dettagli
column-size = Dimensione
column-type = Tipo
column-modified = Modificata
sort-selector = Ordina per
sort-menu-name = Nome
sort-menu-size = Dimensione
sort-menu-type = Tipo
sort-menu-modified = Modificata
sort-menu-toggle-direction = Inverti ordinamento
zoom-out = Riduci icone
zoom-in = Ingrandisci icone
zoom-selector = Dimensione icone
menu-about = Informazioni su Kito Files
menu-preferences = Preferenze…

## Barra del percorso
path-bar-name = Percorso
path-bar-description = Fai clic nell’area vuota o premi Ctrl+L per modificare il percorso
path-placeholder = Digita un percorso, Invio per andare
path-suggestions = Suggerimenti di percorso
crumb-edit-current = Fai clic per modificare il percorso
crumb-edit-path = Modifica percorso
crumb-root = Radice del filesystem

## Informazioni
about-comments = Un file manager Wayland leggero in Rust + GTK4
about-developer = Collaboratori di Kito Files

## Barra laterale
side-places = Luoghi
side-devices = Dispositivi
side-network = Rete
side-trash = Cestino
side-remove = Rimuovi dai luoghi
side-browse-network = Esplora rete
side-op-failed = Operazione non riuscita
side-home = Cartella personale
side-desktop = Scrivania
side-documents = Documenti
side-downloads = Scaricati
side-music = Musica
side-pictures = Immagini
side-videos = Video
side-filesystem = File system

## Elenco dei file
column-name = Nome

## Barra di stato
status-items =
    { $count ->
        [one] { $count } elemento
       *[other] { $count } elementi
    }
status-selected =
    { $count ->
        [one] { $count } elemento selezionato
       *[other] { $count } elementi selezionati
    }

## Schede
error-open-folder = Impossibile aprire la cartella
error-invalid-path = Inserisci un percorso locale o un URI da aprire.
empty-folder = Questa cartella è vuota
loading-folder = Caricamento…
loading-interrupt = Interrompi
monitor-unavailable = Aggiornamenti live non disponibili per questa posizione. Premi F5 per aggiornare.
drop-trash-copy-unsupported = Non è possibile copiare elementi nel cestino. Scegli Sposta.
monitor-refresh-error = Aggiornamento live non riuscito. Premi F5 per aggiornare la cartella.
error-open-file = Impossibile aprire il file
error-open-file-detail = Kito Files non è riuscito ad avviare l’applicazione predefinita: { $error }

## Notifiche appunti
toast-nothing = Nessuna selezione
toast-cut = Tagliato
toast-copied = Copiato
clip-empty = Appunti vuoti
clip-changed = Appunti modificati, riprova
clip-moving = Spostamento già in corso
clip-nofiles = Gli appunti non contengono file
pasted-items =
    { $count ->
        [one] Incollato { $count } elemento
       *[other] Incollati { $count } elementi
    }
paste-failed =
    { $failed ->
        [one] Impossibile incollare { $failed } elemento su { $total }
       *[other] Impossibile incollare { $failed } elementi su { $total }
    }
moved-trash =
    { $count ->
        [one] Spostato { $count } elemento nel cestino
       *[other] Spostati { $count } elementi nel cestino
    }
moved-trash-failed =
    { $failed ->
        [one] Impossibile spostare { $failed } elemento su { $total } nel cestino
       *[other] Impossibile spostare { $failed } elementi su { $total } nel cestino
    }
restored-items =
    { $count ->
        [one] Ripristinato { $count } elemento
       *[other] Ripristinati { $count } elementi
    }
restored-failed =
    { $failed ->
        [one] Impossibile ripristinare { $failed } elemento su { $total }
       *[other] Impossibile ripristinare { $failed } elementi su { $total }
    }
trash-removed =
    { $count ->
        [one] Rimosso { $count } elemento dal cestino
       *[other] Rimossi { $count } elementi dal cestino
    }
trash-remove-failed =
    { $failed ->
        [one] Impossibile rimuovere { $failed } elemento su { $total } dal cestino
       *[other] Impossibile rimuovere { $failed } elementi su { $total } dal cestino
    }
deleted-items =
    { $count ->
        [one] Eliminato { $count } elemento
       *[other] Eliminati { $count } elementi
    }
delete-failed-items =
    { $failed ->
        [one] Impossibile eliminare { $failed } elemento su { $total }
       *[other] Impossibile eliminare { $failed } elementi su { $total }
    }
pin-select-folder = Seleziona una sola cartella da aggiungere
pin-unpinned = Rimosso dai luoghi
pin-pinned = Aggiunto ai luoghi
rename-select = Seleziona un solo elemento da rinominare
properties-select-one = Seleziona un solo elemento per mostrarne le proprietà
renamed-ok = Rinominato
folder-created = Cartella creata
created-file = File creato: { $name }

## Risultati delle operazioni
operation-copy = Copia
operation-move = Spostamento
operation-trash = Spostamento nel cestino
operation-restore = Ripristino
operation-delete = Eliminazione definitiva
operation-empty-trash = Svuotamento del cestino
operation-rename = Rinomina
operation-create-folder = Creazione cartella
operation-create-file = Creazione file
operation-result-title = Risultato dell’operazione
operation-result-success = { $operation }: { $count ->
    [one] completato un elemento.
   *[other] completati { $count } elementi.
}
operation-result-failed = { $operation }: tutti i { $total } elementi non sono riusciti.
operation-result-partial = { $operation }: { $succeeded } su { $total } riusciti; { $failed } non riusciti.
operation-result-cancelled = { $operation }: { $succeeded } riusciti, { $failed } non riusciti e { $cancelled } annullati su { $total }.
operation-details = Dettagli
operation-detail-operation = Operazione: { $operation }
operation-detail-source = Origine: { $source }
operation-detail-destination = Destinazione: { $destination }
operation-item-succeeded = Completato
operation-item-failed = Non riuscito: { $error }
operation-item-cancelled = Annullato: { $error }
operation-retry-failed = Riprova gli elementi non riusciti
operation-close = Chiudi
trash-already-empty = Il cestino è già vuoto.

## Finestre di dialogo
dialog-ok = OK
dialog-cancel = Annulla
trash-empty-title = Svuotare il cestino?
trash-empty-body = Tutti gli elementi nel cestino saranno eliminati definitivamente.
trash-empty-confirm = Svuota cestino
delete-title = Eliminare definitivamente?
delete-body =
    { $count ->
        [one] { $count } elemento sarà eliminato. Non si può annullare.
       *[other] { $count } elementi saranno eliminati. Non si può annullare.
    }
delete-confirm = Elimina
rename-title = Rinomina
rename-placeholder = Nome file
rename-confirm = Rinomina
new-folder-title = Nuova cartella
new-folder-placeholder = Nome cartella
new-folder-initial = Cartella senza titolo
new-folder-confirm = Crea
new-file-title = Nuovo file
new-file-placeholder = Nome file
new-file-confirm = Crea
error-paste = Impossibile incollare
error-restore = Impossibile ripristinare
error-trash = Impossibile svuotare il cestino
error-unpin = Impossibile rimuovere
error-pin = Impossibile aggiungere
error-rename = Impossibile rinominare
error-create-folder = Impossibile creare la cartella
error-create-file = Impossibile creare il file
error-props = Impossibile leggere le proprietà
error-terminal = Impossibile aprire il terminale
error-terminal-root = Impossibile aprire il terminale root

## Proprietà
props-title = Proprietà
props-folder = Cartella
props-file = File
props-type = Tipo
props-size = Dimensione
props-size-items =
    { $count ->
        [one] { $count } elemento
       *[other] { $count } elementi
    }
props-location = Posizione
props-modified = Modificata
props-close = Chiudi

## Terminale
term-no-term = Nessun emulatore di terminale trovato
term-no-root = Nessun terminale installato supporta la shell root
term-cannot-here = Impossibile aprire il terminale qui
term-local-only = Solo cartelle locali sono supportate.
term-selected-unsupported = L’emulatore di terminale selezionato ({ $terminal }) non è supportato.
term-selected-no-root = { $terminal } non consente di aprire una shell root.
term-launch-error = Impossibile avviare il terminale: { $error }

## Nomi suggeriti
suggest-folder = Nuova cartella
suggest-text-file = Nuovo file di testo
suggest-empty-file = Nuovo file
suggest-word-doc = Nuovo documento Word
suggest-spreadsheet = Nuovo foglio di calcolo
suggest-html = Nuova pagina HTML

## Preferenze
prefs-title = Preferenze
prefs-general = Generale
prefs-integration = Integrazione
prefs-language = Lingua
prefs-default-view = Vista predefinita
prefs-new-tabs-note = Si applica alle nuove schede
prefs-open-items = Apertura degli elementi
prefs-double-click = Doppio clic
prefs-single-click = Clic singolo
prefs-lang-system = Lingua del sistema
prefs-lang-english = English
prefs-lang-italian = Italiano
prefs-language-applied = La lingua viene applicata immediatamente.
prefs-terminal = Terminale
prefs-terminal-automatic = Automatico
prefs-terminal-missing = { $terminal } non è disponibile; verrà usato il rilevamento automatico.
prefs-save-error = Impossibile salvare le preferenze: { $error }
prefs-window-controls = Controlli della finestra
prefs-follow-system = Segui le impostazioni di sistema
prefs-show-minimize = Mostra il pulsante per ridurre a icona
prefs-show-maximize = Mostra il pulsante per massimizzare
prefs-show-close = Mostra il pulsante per chiudere
