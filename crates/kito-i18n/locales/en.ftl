## Context menu
menu-open = Open
menu-open-new-tab = Open in New Tab
menu-open-new-window = Open in New Window
open-folders-only = Select folders to open in new tabs or a new window
menu-pin = Add Freedesktop bookmark
menu-unpin = Remove Freedesktop bookmark
menu-cut = Cut
menu-copy = Copy
menu-paste = Paste
menu-rename = Rename…
menu-trash = Move to Trash
menu-delete = Delete Permanently…
menu-properties = Properties
menu-select-all = Select All
menu-invert-selection = Invert Selection
menu-deselect-all = Deselect All
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
bg-new-folder = New Folder…
bg-new-file = New File…
bg-folder-properties = Folder Properties

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
view-columns = Details Columns
column-size = Size
column-type = Type
column-modified = Modified
sort-selector = Sorting
sort-menu-name = Name
sort-menu-size = Size
sort-menu-type = Type
sort-menu-modified = Modified
sort-menu-toggle-direction = Reverse sort order
zoom-out = Zoom out
zoom-in = Zoom in
zoom-selector = Icon size
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
side-favorites = Favorites
side-bookmarks = Bookmarks
side-add-bookmark = Add bookmark
menu-add-favorite = Add to Favorites
menu-remove-favorite = Remove from Favorites
favorite-added = Added to Favorites
favorite-removed = Removed from Favorites
error-favorite = Could not update Favorites
error-migrate-bookmarks = Could not migrate saved locations to Bookmarks
favorites-not-a-folder = Favorites is a virtual view, not a destination folder.
side-favorites-toggle = Show or hide folder bookmarks
side-favorites-empty = No additional folder bookmarks
sidebar-search = Search
sidebar-search-unavailable = Search will be available later
side-trash = Trash
side-remove-bookmark = Remove from Bookmarks
side-remove-legacy-location = Remove legacy location
side-rename-bookmark = Rename…
side-bookmark-name = Bookmark name
side-bookmark-save = Save
side-bookmark-name-empty = Enter a name for the bookmark.
side-bookmark-name-duplicate = A bookmark with this name already exists.
side-bookmark-rename-error = Could not rename bookmark: { $error }
side-bookmark-current = Add Current Folder to Bookmarks
side-bookmark-added = Added to Bookmarks
side-bookmark-exists = This location is already bookmarked.
side-bookmark-unavailable = This location cannot be added to Bookmarks.
side-bookmark-error = Could not add bookmark
side-browse-network = Browse network
side-op-failed = Operation failed
side-eject = Eject
side-unmount = Unmount
side-home = Home
side-desktop = Desktop
side-documents = Documents
side-downloads = Downloads
side-music = Music
side-pictures = Pictures
side-videos = Videos
side-filesystem = File System
side-custom = Custom Locations

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
empty-folder-description = Add files to this folder, or choose another location.
empty-favorites = No favorites yet
empty-favorites-description = Add files or folders to Favorites to find them here.
loading-folder = Loading…
loading-interrupt = Stop
monitor-unavailable = Live updates are unavailable for this location. Press F5 to refresh.
drop-trash-copy-unsupported = Copying items to Trash is unavailable. Choose Move instead.
monitor-refresh-error = Live update failed. Press F5 to refresh this folder.
error-open-file = Could not open file
error-open-file-detail = Kito Files could not start the default application: { $error }

## Clipboard toasts
toast-nothing = Nothing selected
toast-cut = Cut
toast-copied = Copied
undo-action = Undo
undo-empty = Nothing to undo
undo-in-progress = Wait for the current file operation to finish before undoing
undo-success = Undone
undo-conflict = Undo could not be completed safely. The item or its original location has changed.
clip-empty = Clipboard is empty
clip-changed = Clipboard changed, try again
clip-moving = Move already in progress
clip-nofiles = Clipboard has no files to paste
clip-publish-failed = Could not publish to the clipboard
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
pin-select-item = Select a single file or folder to pin
pin-unpinned = Unpinned from Places
pin-pinned = Pinned to Places
rename-select = Select a single item to rename
properties-select-one = Select a single item to show its properties
renamed-ok = Renamed
folder-created = Folder created
created-file = Created { $name }

## Operation results
operation-copy = Copy
operation-move = Move
operation-trash = Move to Trash
operation-restore = Restore
operation-delete = Delete permanently
operation-empty-trash = Empty Trash
operation-rename = Rename
operation-create-folder = Create folder
operation-create-file = Create file
operation-result-title = Operation result
operation-worker-failed = The file operation could not be completed safely. Check the affected files.
operation-queue-full = Operations queue is full; try again later.
operation-result-success = { $operation }: { $count ->
    [one] completed one item.
   *[other] completed { $count } items.
}
operation-result-failed = { $operation }: all { $total } items failed.
operation-result-partial = { $operation }: { $succeeded } of { $total } succeeded; { $failed } failed.
operation-result-cancelled = { $operation }: { $succeeded } succeeded, { $failed } failed, and { $cancelled } cancelled out of { $total }.
operation-details = Details
operation-detail-operation = Operation: { $operation }
operation-detail-source = Source: { $source }
operation-detail-destination = Destination: { $destination }
operation-item-succeeded = Completed
operation-item-failed = Failed: { $error }
operation-item-cancelled = Cancelled: { $error }
operation-retry-failed = Retry failed items
operation-close = Close

## F08 Operations center
jobs-title = Operations
jobs-destination = Destination: { $destination }
jobs-cancel = Cancel
jobs-cancelling = Cancelling…
jobs-progress-files = { $done } of { $total }
jobs-status-queued = Queued
jobs-status-running = Running
jobs-status-cancelling = Cancelling
jobs-status-completed = Completed
jobs-status-partial = Partial
jobs-status-failed = Failed
jobs-status-cancelled = Cancelled
jobs-cancelled-before-start = Cancelled before the operation started
jobs-item-succeeded = Completed: { $source }
jobs-item-failed = Failed: { $source } — { $error }
jobs-item-cancelled = Cancelled: { $source } — { $error }
menu-operations = Operations
trash-already-empty = The Trash is already empty.

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
prefs-status-bar = Status bar
prefs-status-item-count = Show item count
prefs-status-zoom-controls = Show zoom controls
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
prefs-shortcuts = Keyboard shortcuts
prefs-shortcuts-group = Kito shortcuts
prefs-shortcuts-note = Select Edit, then press a key combination. Escape cancels; printable keys need a modifier.
prefs-shortcuts-reset-all = Reset all
prefs-shortcuts-edit = Edit
prefs-shortcuts-clear = Disable
prefs-shortcuts-reset = Reset
prefs-shortcuts-disabled = Disabled
prefs-shortcuts-capture-title = Press a shortcut
prefs-shortcuts-capture-note = Press a key combination. Escape cancels.
prefs-shortcuts-conflict = Already used by { $action }
shortcut-copy = Copy
shortcut-cut = Cut
shortcut-paste = Paste
shortcut-trash = Move to Trash
shortcut-delete = Delete permanently
shortcut-rename = Rename
shortcut-edit-path = Edit location
shortcut-reload-f5 = Reload (F5)
shortcut-reload-ctrl-r = Reload (Ctrl+R)
shortcut-new-folder = Create folder
shortcut-preferences = Preferences
shortcut-undo = Undo
shortcut-select-all = Select all
shortcut-bookmark-current = Add current folder to bookmarks
shortcut-new-tab = New tab
shortcut-close-tab = Close tab
shortcut-reopen-tab = Reopen closed tab
shortcut-next-tab = Next tab
shortcut-previous-tab = Previous tab
shortcut-back = Back
shortcut-forward = Forward
shortcut-up = Parent folder
shortcut-zoom-in-plus = Zoom in (plus)
shortcut-zoom-in-keypad = Zoom in (keypad plus)
shortcut-zoom-out-minus = Zoom out (minus)
shortcut-zoom-out-keypad = Zoom out (keypad minus)
shortcut-zoom-reset = Reset zoom
