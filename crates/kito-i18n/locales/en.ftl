## Context menu
menu-open = Open
menu-pin = Pin to Places
menu-cut = Cut
menu-copy = Copy
menu-paste = Paste
menu-rename = Rename…
menu-trash = Move to Trash
menu-delete = Delete Permanently…
menu-properties = Properties
menu-restore = Restore
menu-create = Create
menu-new-folder = New Folder
menu-new-text-file = New Text File
menu-new-empty-file = New Empty File
menu-new-word-doc = Word Document
menu-new-spreadsheet = Spreadsheet
menu-new-html = HTML Page
menu-open-terminal = Open Terminal
menu-open-terminal-root = Open Terminal as Root
menu-empty-trash = Empty Trash…

## Header bar
nav-back = Back
nav-forward = Forward
nav-up = Go up
nav-new-tab = New tab
menu-application = Application menu
view-selector = View options
view-current = Current view: { $view }
view-icons = Icons
view-compact = Compact
view-details = Details
view-hidden = Show Hidden Files
menu-about = About Kito Files
menu-preferences = Preferences…

## Path bar
path-bar-name = Location
path-bar-description = Click an empty area or press Ctrl+L to edit the path
path-placeholder = Type a path, Enter to go
path-suggestions = Path suggestions
crumb-edit-current = Click to edit path
crumb-edit-path = Edit path
crumb-root = Filesystem root

## About
about-comments = A lightweight Wayland file manager in Rust + GTK4
about-developer = Kito Files contributors
test-only-english = English fallback

## Sidebar
side-places = Places
side-devices = Devices
side-network = Network
side-trash = Trash
side-remove = Remove from Places
side-browse-network = Browse network
side-op-failed = Operation failed
side-home = Home
side-desktop = Desktop
side-documents = Documents
side-downloads = Downloads
side-music = Music
side-pictures = Pictures
side-videos = Videos
side-filesystem = File System

## File list
column-name = Name

## Status bar
status-items =
    { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
status-selected =
    { $count ->
        [one] { $count } item selected
       *[other] { $count } items selected
    }

## Tabs
error-open-folder = Could not open folder
error-invalid-path = Enter a local path or URI to open.
empty-folder = This folder is empty
error-open-file = Could not open file
error-open-file-detail = Kito Files could not start the default application: { $error }

## Clipboard toasts
toast-nothing = Nothing selected
toast-cut = Cut
toast-copied = Copied
clip-empty = Clipboard is empty
clip-changed = Clipboard changed, try again
clip-moving = Move already in progress
clip-nofiles = Clipboard has no files to paste
pasted-items =
    { $count ->
        [one] Pasted { $count } item
       *[other] Pasted { $count } items
    }
paste-failed =
    { $failed ->
        [one] Failed to paste { $failed } of { $total } items
       *[other] Failed to paste { $failed } of { $total } items
    }
moved-trash =
    { $count ->
        [one] Moved { $count } item to trash
       *[other] Moved { $count } items to trash
    }
moved-trash-failed =
    { $failed ->
        [one] Failed to move { $failed } of { $total } items to Trash
       *[other] Failed to move { $failed } of { $total } items to Trash
    }
restored-items =
    { $count ->
        [one] Restored { $count } item
       *[other] Restored { $count } items
    }
restored-failed =
    { $failed ->
        [one] Could not restore { $failed } of { $total } items
       *[other] Could not restore { $failed } of { $total } items
    }
trash-removed =
    { $count ->
        [one] Removed { $count } item from the Trash
       *[other] Removed { $count } items from the Trash
    }
trash-remove-failed =
    { $failed ->
        [one] Could not remove { $failed } of { $total } items from the Trash
       *[other] Could not remove { $failed } of { $total } items from the Trash
    }
deleted-items =
    { $count ->
        [one] Deleted { $count } item
       *[other] Deleted { $count } items
    }
delete-failed-items =
    { $failed ->
        [one] Failed to delete { $failed } of { $total } items
       *[other] Failed to delete { $failed } of { $total } items
    }
pin-select-folder = Select a single folder to pin
pin-unpinned = Unpinned from Places
pin-pinned = Pinned to Places
rename-select = Select a single item to rename
renamed-ok = Renamed
folder-created = Folder created
created-file = Created { $name }

## Dialogs
dialog-ok = Ok
dialog-cancel = Cancel
trash-empty-title = Empty the Trash?
trash-empty-body = All items in the Trash will be permanently deleted.
trash-empty-confirm = Empty Trash
delete-title = Delete permanently?
delete-body =
    { $count ->
        [one] { $count } item will be deleted. This cannot be undone.
       *[other] { $count } items will be deleted. This cannot be undone.
    }
delete-confirm = Delete
rename-title = Rename
rename-placeholder = File name
rename-confirm = Rename
new-folder-title = New Folder
new-folder-placeholder = Folder name
new-folder-initial = Untitled Folder
new-folder-confirm = Create
new-file-title = New File
new-file-placeholder = File name
new-file-confirm = Create
error-paste = Could not paste
error-restore = Could not restore
error-trash = Could not empty the Trash
error-unpin = Could not unpin
error-pin = Could not pin
error-rename = Could not rename
error-create-folder = Could not create folder
error-create-file = Could not create file
error-props = Could not read properties
error-terminal = Could not open a terminal
error-terminal-root = Could not open a root terminal

## Properties
props-title = Properties
props-folder = Folder
props-file = File
props-type = Type
props-size = Size
props-size-items =
    { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
props-location = Location
props-modified = Modified
props-close = Close

## Terminal
term-no-term = No terminal emulator found
term-no-root = No installed terminal supports a root shell
term-cannot-here = Cannot open a terminal here
term-local-only = Only local folders are supported.
term-selected-unsupported = The selected terminal emulator ({ $terminal }) is not supported.
term-selected-no-root = { $terminal } does not support opening a root shell.
term-launch-error = The terminal could not be started: { $error }

## Suggested names
suggest-folder = New Folder
suggest-text-file = New Text File
suggest-empty-file = New File
suggest-word-doc = New Word Document
suggest-spreadsheet = New Spreadsheet
suggest-html = New HTML Page

## Preferences
prefs-title = Preferences
prefs-general = General
prefs-integration = Integration
prefs-language = Language
prefs-default-view = Default view
prefs-new-tabs-note = Applies to new tabs
prefs-open-items = Open items
prefs-double-click = Double click
prefs-single-click = Single click
prefs-lang-system = System language
prefs-lang-english = English
prefs-lang-italian = Italiano
prefs-language-applied = Language changes apply immediately.
prefs-terminal = Terminal
prefs-terminal-automatic = Automatic
prefs-terminal-missing = { $terminal } is unavailable; using Automatic.
prefs-save-error = Could not save preferences: { $error }
prefs-window-controls = Window Controls
prefs-follow-system = Follow system settings
prefs-show-minimize = Show minimize button
prefs-show-maximize = Show maximize button
prefs-show-close = Show close button
