//! Classic views: icons, compact, details. One row GObject + factory
//! on a shared ListStore; no widget subclassing.

use glib::subclass::prelude::ObjectSubclassIsExt as _;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

const THUMB_MAX_INPUT: u64 = 20 * 1024 * 1024;
const THUMB_MAX_DIMENSION: i32 = 4096;
const THUMB_MAX_PIXELS: i64 = 16 * 1024 * 1024;
const THUMB_CACHE_LIMIT: usize = 128;
// Bound decoded pixel memory as well as entry count (negative entries use zero bytes).
const THUMB_CACHE_BYTE_LIMIT: usize = 32 * 1024 * 1024;
const THUMB_MAX_ACTIVE: usize = 4;
const THUMB_MAX_WAITING: usize = 256;

#[derive(Clone)]
struct ThumbnailPixels {
    data: glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
}

#[derive(Default)]
struct ThumbnailBinding {
    generation: std::cell::Cell<u64>,
    uri: std::cell::RefCell<String>,
}

#[derive(Clone)]
struct ThumbnailTarget {
    image: glib::WeakRef<gtk::Image>,
    binding: Rc<ThumbnailBinding>,
    generation: u64,
    uri: String,
    max_size: i32,
}

impl ThumbnailTarget {
    fn current_image(&self) -> Option<gtk::Image> {
        let image = self.image.upgrade()?;
        current_binding(&self.binding, self.generation, &self.uri).then_some(image)
    }
}

struct ThumbnailQueue<T> {
    active: usize,
    order: VecDeque<String>,
    waiting: HashMap<String, Vec<T>>,
    running: HashMap<String, Vec<T>>,
}

trait ThumbnailQueueTarget {
    fn same_widget(&self, other: &Self) -> bool;
    fn is_live(&self) -> bool;
}

impl ThumbnailQueueTarget for ThumbnailTarget {
    fn same_widget(&self, other: &Self) -> bool {
        match (self.image.upgrade(), other.image.upgrade()) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    }

    fn is_live(&self) -> bool {
        self.current_image().is_some()
    }
}

impl<T> Default for ThumbnailQueue<T> {
    fn default() -> Self {
        Self {
            active: 0,
            order: VecDeque::new(),
            waiting: HashMap::new(),
            running: HashMap::new(),
        }
    }
}

impl<T> ThumbnailQueue<T> {
    fn enqueue(&mut self, key: String, target: T) -> bool
    where
        T: ThumbnailQueueTarget,
    {
        // Remove dead/recycled targets across every key before accepting another
        // request. Running jobs remain active even if no consumer remains.
        for targets in self.waiting.values_mut().chain(self.running.values_mut()) {
            targets.retain(ThumbnailQueueTarget::is_live);
        }
        let empty_waiting: Vec<_> = self
            .waiting
            .iter()
            .filter_map(|(key, targets)| targets.is_empty().then_some(key.clone()))
            .collect();
        for old_key in empty_waiting {
            self.waiting.remove(&old_key);
            self.order.retain(|queued| queued != &old_key);
        }
        // A widget can have only one outstanding destination. Remove it from
        // older jobs before inserting the newest binding, without cancelling
        // shared/running work needed by other widgets.
        for targets in self.waiting.values_mut().chain(self.running.values_mut()) {
            targets.retain(|old| !old.same_widget(&target));
        }
        let empty_waiting: Vec<_> = self
            .waiting
            .iter()
            .filter_map(|(key, targets)| targets.is_empty().then_some(key.clone()))
            .collect();
        for old_key in empty_waiting {
            self.waiting.remove(&old_key);
            self.order.retain(|queued| queued != &old_key);
        }
        if let Some(targets) = self.running.get_mut(&key) {
            targets.push(target);
            return true;
        }
        if let Some(targets) = self.waiting.get_mut(&key) {
            targets.push(target);
            return true;
        }
        if self.waiting.len() >= THUMB_MAX_WAITING {
            return false;
        }
        self.order.push_back(key.clone());
        self.waiting.insert(key, vec![target]);
        true
    }

    fn start_next(&mut self) -> Option<(String, T)>
    where
        T: Clone,
    {
        if self.active >= THUMB_MAX_ACTIVE {
            return None;
        }
        let key = self.order.pop_front()?;
        let targets = self.waiting.remove(&key)?;
        let first = targets.first()?.clone();
        self.running.insert(key.clone(), targets);
        self.active += 1;
        Some((key, first))
    }

    fn targets_mut(&mut self, key: &str) -> Option<&mut Vec<T>> {
        self.running.get_mut(key)
    }

    fn finish(&mut self, key: &str) -> Vec<T> {
        self.active = self.active.saturating_sub(1);
        self.running.remove(key).unwrap_or_default()
    }
}

thread_local! {
    static THUMB_CACHE: std::cell::RefCell<std::collections::VecDeque<(String, Option<ThumbnailPixels>)>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static THUMB_QUEUE: std::cell::RefCell<ThumbnailQueue<ThumbnailTarget>> = std::cell::RefCell::new(ThumbnailQueue::default());
}

fn trim_thumbnail_cache(cache: &mut VecDeque<(String, Option<ThumbnailPixels>)>) {
    let mut bytes: usize = cache
        .iter()
        .filter_map(|(_, pixels)| pixels.as_ref())
        .map(|pixels| pixels.data.len())
        .sum();
    while cache.len() > THUMB_CACHE_LIMIT || bytes > THUMB_CACHE_BYTE_LIMIT {
        let Some((_, removed)) = cache.pop_back() else {
            break;
        };
        if let Some(pixels) = removed {
            bytes = bytes.saturating_sub(pixels.data.len());
        }
    }
}

use crate::dnd::{self, DropHandler};
pub use crate::preferences::model::ViewMode;
use crate::preferences::model::{DetailColumnWidths, OpenItems};

const ICON_VIEW_BASE_SIZE: i32 = 96;
const ICON_VIEW_BASE_CELL_WIDTH: i32 = 160;
const NAME_ICON_MIN_SIZE: i32 = 26;

fn name_icon_pixel_size(base_size: i32, zoom: u8) -> i32 {
    (base_size * i32::from(zoom) / 100).max(NAME_ICON_MIN_SIZE)
}

fn icon_pixel_size(zoom: u8) -> i32 {
    (ICON_VIEW_BASE_SIZE * i32::from(zoom) / 100).max(24)
}

fn icon_cell_width(zoom: u8) -> i32 {
    (ICON_VIEW_BASE_CELL_WIDTH * i32::from(zoom) / 100).max(96)
}

#[derive(Clone, Copy)]
pub struct ViewDisplay {
    pub icon_zoom: u8,
    pub show_size: bool,
    pub show_type: bool,
    pub show_modified: bool,
    pub detail_column_widths: DetailColumnWidths,
}

/// Right-click on a row: object + point + anchor widget.
/// The handler selects the object and opens the menu.
pub type SecondaryHandler = Rc<dyn Fn(&FileObject, f64, f64, &gtk::Widget)>;
pub type MiddleHandler = Rc<dyn Fn(&FileObject)>;
pub type DragPrepareHandler = Rc<dyn Fn(&FileObject) -> Option<dnd::PreparedDrag>>;

pub struct ViewHandlers<'a> {
    pub on_secondary: &'a SecondaryHandler,
    pub on_middle: &'a MiddleHandler,
    pub on_drag_prepare: &'a DragPrepareHandler,
    pub on_drop: &'a DropHandler,
    pub on_external_move: &'a dnd::ExternalMoveHandler,
}

/// Binds right-click to the row for the current object. Called on every
/// bind (rows are recycled): removes the previous gesture.
fn set_secondary(row: &impl IsA<gtk::Widget>, obj: &FileObject, on_secondary: &SecondaryHandler) {
    let controllers = row.observe_controllers();
    let mut kept = 0;
    while kept < controllers.n_items() {
        let Some(controller) = controllers
            .item(kept)
            .and_downcast::<gtk::EventController>()
        else {
            kept += 1;
            continue;
        };
        if controller
            .clone()
            .downcast::<gtk::GestureClick>()
            .is_ok_and(|click| click.button() == gtk::gdk::BUTTON_SECONDARY)
        {
            row.remove_controller(&controller);
        } else {
            kept += 1;
        }
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let obj = obj.clone();
    let on_secondary = on_secondary.clone();
    gesture.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(source) = gesture.widget() else {
            return;
        };
        on_secondary(&obj, x, y, &source);
    });
    row.add_controller(gesture);
}

fn set_middle(row: &impl IsA<gtk::Widget>, obj: &FileObject, on_middle: &MiddleHandler) {
    let controllers = row.observe_controllers();
    let mut kept = 0;
    while kept < controllers.n_items() {
        let Some(controller) = controllers
            .item(kept)
            .and_downcast::<gtk::EventController>()
        else {
            kept += 1;
            continue;
        };
        if controller
            .clone()
            .downcast::<gtk::GestureClick>()
            .is_ok_and(|click| click.button() == gtk::gdk::BUTTON_MIDDLE)
        {
            row.remove_controller(&controller);
        } else {
            kept += 1;
        }
    }
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_MIDDLE)
        .build();
    let obj = obj.clone();
    let on_middle = on_middle.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if obj.is_dir() {
            on_middle(&obj);
        }
    });
    row.add_controller(gesture);
}

mod imp {
    use glib::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct FileObject {
        pub name: RefCell<String>,
        pub uri: RefCell<String>,
        pub is_dir: Cell<bool>,
        pub size: Cell<i64>,
        pub modified: Cell<Option<i64>>,
        pub content_type: RefCell<Option<String>>,
        pub icon: RefCell<Option<gio::Icon>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileObject {
        const NAME: &'static str = "KitoFilesFileObject";
        type Type = super::FileObject;
    }

    impl ObjectImpl for FileObject {}
}

glib::wrapper! {
    pub struct FileObject(ObjectSubclass<imp::FileObject>);
}

impl FileObject {
    pub fn new(entry: &kito_core::Entry) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        *imp.name.borrow_mut() = entry.name.clone();
        *imp.uri.borrow_mut() = entry.uri.clone();
        imp.is_dir.set(entry.is_dir);
        imp.size.set(entry.size);
        imp.modified.set(entry.modified);
        *imp.content_type.borrow_mut() = entry.content_type.clone();
        // Serialized in the worker; parsed here on the main thread.
        // Unparsable values fall back to the generic icon in the factories.
        *imp.icon.borrow_mut() = entry
            .icon
            .as_deref()
            .and_then(|s| gio::Icon::for_string(s).ok());
        obj
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn uri(&self) -> String {
        self.imp().uri.borrow().clone()
    }

    pub fn is_dir(&self) -> bool {
        self.imp().is_dir.get()
    }

    pub fn size(&self) -> i64 {
        self.imp().size.get()
    }

    pub fn modified(&self) -> Option<i64> {
        self.imp().modified.get()
    }

    pub fn type_label(&self) -> String {
        if self.is_dir() {
            return crate::l10n::tr("props-folder");
        }
        match self.imp().content_type.borrow().as_deref() {
            Some(mime) => gio::content_type_get_description(mime).to_string(),
            None => crate::l10n::tr("props-file"),
        }
    }

    pub fn icon(&self) -> Option<gio::Icon> {
        self.imp().icon.borrow().clone()
    }
}

/// 1234567 -> "1.2 MB", -1 (folders) -> "—".
pub fn human_size(bytes: i64) -> String {
    if bytes < 0 {
        return "—".to_string();
    }
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn obj_of(list_item: &gtk::ListItem) -> FileObject {
    list_item.item().and_downcast::<FileObject>().unwrap()
}

/// Content fallback only. File and folder icons in the main views keep the
/// normal GIO theme icons; do not route them through interface-control helpers.
fn fallback_icon(is_dir: bool) -> gio::ThemedIcon {
    gio::ThemedIcon::new(if is_dir { "folder" } else { "text-x-generic" })
}

fn thumbnail_eligible(obj: &FileObject) -> bool {
    !obj.is_dir()
        && obj.size() >= 0
        && obj.size() as u64 <= THUMB_MAX_INPUT
        && obj
            .imp()
            .content_type
            .borrow()
            .as_deref()
            .is_some_and(|mime| mime.starts_with("image/"))
        && gio::File::for_uri(&obj.uri()).path().is_some()
}

fn thumbnail_key(obj: &FileObject, size: i32) -> String {
    format!(
        "{}:{}:{}:{size}",
        obj.uri(),
        obj.size(),
        obj.modified().unwrap_or_default()
    )
}

fn cached_thumbnail(key: &str) -> Option<Option<ThumbnailPixels>> {
    THUMB_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(k, _)| k == key)?;
        let entry = cache.remove(index)?;
        let value = entry.1.clone();
        cache.push_front(entry);
        Some(value)
    })
}

fn remember_thumbnail(key: String, value: Option<ThumbnailPixels>) {
    THUMB_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(k, _)| k != &key);
        cache.push_front((key, value));
        trim_thumbnail_cache(&mut cache);
    });
}

fn decode_thumbnail(uri: String, max_size: i32) -> Option<ThumbnailPixels> {
    let path = gio::File::for_uri(&uri).path()?;
    // Preflight avoids opening known special files. On Unix, O_NONBLOCK closes
    // the preflight/open race for FIFOs; fstat then validates the opened inode.
    if !std::fs::metadata(&path).ok()?.is_file() {
        return None;
    }
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .ok()?
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > THUMB_MAX_INPUT {
        return None;
    }
    use std::io::Read as _;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(THUMB_MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > THUMB_MAX_INPUT {
        return None;
    }
    let loader = gdk_pixbuf::PixbufLoader::new();
    let oversized = std::rc::Rc::new(std::cell::Cell::new(false));
    let oversized_signal = oversized.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0
            || height <= 0
            || width > THUMB_MAX_DIMENSION
            || height > THUMB_MAX_DIMENSION
            || i64::from(width) * i64::from(height) > THUMB_MAX_PIXELS
        {
            oversized_signal.set(true);
            // GdkPixbuf emits this before allocating the decoded surface.
            loader.set_size(1, 1);
        }
    });
    loader.write(&bytes).ok()?;
    loader.close().ok()?;
    if oversized.get() {
        return None;
    }
    let source = loader.pixbuf()?;
    let (width, height) = (source.width(), source.height());
    if width <= 0
        || height <= 0
        || width > THUMB_MAX_DIMENSION
        || height > THUMB_MAX_DIMENSION
        || i64::from(width) * i64::from(height) > THUMB_MAX_PIXELS
    {
        return None;
    }
    let scale = (max_size as f64 / f64::from(width.max(height))).min(1.0);
    let width = ((f64::from(width) * scale).round() as i32).max(1);
    let height = ((f64::from(height) * scale).round() as i32).max(1);
    let thumb = source.scale_simple(width, height, gdk_pixbuf::InterpType::Bilinear)?;
    Some(ThumbnailPixels {
        data: thumb.read_pixel_bytes(),
        width,
        height,
        rowstride: thumb.rowstride(),
        alpha: thumb.has_alpha(),
    })
}

fn current_binding(binding: &ThumbnailBinding, generation: u64, uri: &str) -> bool {
    binding.generation.get() == generation && binding.uri.borrow().as_str() == uri
}

fn invalidate_thumbnail_binding(image: &gtk::Image) {
    let key = glib::Quark::from_str("kito-thumbnail-binding");
    if let Some(binding) = unsafe { image.qdata::<Rc<ThumbnailBinding>>(key) }
        .map(|value| unsafe { value.as_ref().clone() })
    {
        binding
            .generation
            .set(binding.generation.get().wrapping_add(1));
        binding.uri.borrow_mut().clear();
    }
}

fn start_thumbnail(image: &gtk::Image, obj: &FileObject, max_size: i32) {
    let max_size = max_size.saturating_mul(image.scale_factor()).clamp(1, 512);
    let key = thumbnail_key(obj, max_size);
    let uri = obj.uri();
    let qkey = glib::Quark::from_str("kito-thumbnail-binding");
    let binding = unsafe { image.qdata::<Rc<ThumbnailBinding>>(qkey) }
        .map(|value| unsafe { value.as_ref().clone() })
        .unwrap_or_else(|| {
            let binding = Rc::new(ThumbnailBinding::default());
            unsafe {
                image.set_qdata(qkey, binding.clone());
            }
            binding
        });
    let generation = binding.generation.get().wrapping_add(1);
    binding.generation.set(generation);
    *binding.uri.borrow_mut() = uri.clone();
    if let Some(cached) = cached_thumbnail(&key) {
        if let Some(pixels) = cached {
            apply_thumbnail(image, &pixels);
        }
        return;
    }
    let target = ThumbnailTarget {
        image: image.downgrade(),
        binding,
        generation,
        uri,
        max_size,
    };
    // Queue by versioned key: duplicate binds share one decode while retaining
    // all currently bound cells that need the result. Overflow keeps fallback.
    let accepted = THUMB_QUEUE.with(|queue| queue.borrow_mut().enqueue(key, target));
    if accepted {
        pump_thumbnail_queue();
    }
}

fn pump_thumbnail_queue() {
    loop {
        let next = THUMB_QUEUE.with(|queue| {
            let mut queue = queue.borrow_mut();
            queue.start_next()
        });
        let Some((key, _)) = next else {
            break;
        };
        let target = THUMB_QUEUE.with(|queue| {
            let mut queue = queue.borrow_mut();
            let targets = queue.targets_mut(&key)?;
            targets.retain(|target| target.current_image().is_some());
            targets.first().cloned()
        });
        let Some(target) = target else {
            discard_thumbnail_job(&key);
            continue;
        };
        let uri = target.uri.clone();
        let max_size = target.max_size;
        let worker_key = key.clone();
        let task = gio::spawn_blocking(move || decode_thumbnail(uri, max_size));
        glib::MainContext::default().spawn_local(async move {
            let result = task.await.ok().flatten();
            finish_thumbnail_job(&worker_key, result);
        });
    }
}

fn discard_thumbnail_job(key: &str) {
    THUMB_QUEUE.with(|queue| {
        queue.borrow_mut().finish(key);
    });
}

fn finish_thumbnail_job(key: &str, result: Option<ThumbnailPixels>) {
    remember_thumbnail(key.to_string(), result.clone());
    let targets = THUMB_QUEUE.with(|queue| queue.borrow_mut().finish(key));
    for target in targets {
        if let (Some(image), Some(pixels)) = (target.current_image(), result.as_ref()) {
            apply_thumbnail(&image, pixels);
        }
    }
    pump_thumbnail_queue();
}

fn apply_thumbnail(image: &gtk::Image, pixels: &ThumbnailPixels) {
    let texture = gtk::gdk::MemoryTexture::new(
        pixels.width,
        pixels.height,
        if pixels.alpha {
            gtk::gdk::MemoryFormat::R8g8b8a8
        } else {
            gtk::gdk::MemoryFormat::R8g8b8
        },
        &pixels.data,
        pixels.rowstride as usize,
    );
    image.set_paintable(Some(&texture));
}

/// Resolve file-list icons as regular artwork. GtkImage's default GIcon lookup
/// can select a symbolic companion from a GThemedIcon's name/fallback list;
/// the symbolic variants in some themes are monochrome even when the regular
/// icon is not. Keep this scoped to the name column: the grid retains GTK's
/// normal GIcon selection behavior.
fn set_file_list_icon(image: &gtk::Image, icon: &impl IsA<gio::Icon>) {
    let key = glib::Quark::from_str("kito-file-list-icon-state");
    let current_icon = unsafe { image.qdata::<Rc<std::cell::RefCell<Option<gio::Icon>>>>(key) }
        .map(|value| unsafe { value.as_ref().clone() });
    let state = current_icon.unwrap_or_else(|| {
        let state = Rc::new(std::cell::RefCell::new(None));
        unsafe {
            image.set_qdata(key, state.clone());
        }
        install_icon_refresh(image, state.clone());
        state
    });
    *state.borrow_mut() = Some(icon.as_ref().clone());
    refresh_file_list_icon(image, &state);
}

fn refresh_file_list_icon(image: &gtk::Image, state: &Rc<std::cell::RefCell<Option<gio::Icon>>>) {
    let thumbnail_key = glib::Quark::from_str("kito-thumbnail-binding");
    if unsafe { image.qdata::<Rc<ThumbnailBinding>>(thumbnail_key) }
        .map(|value| unsafe { value.as_ref().uri.borrow().is_empty() })
        == Some(false)
    {
        return;
    }
    let Some(icon) = state.borrow().clone() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&image.display());
    let paintable = theme.lookup_by_gicon(
        &icon,
        image.pixel_size().max(1),
        image.scale_factor(),
        image.direction(),
        gtk::IconLookupFlags::FORCE_REGULAR,
    );
    image.set_paintable(Some(&paintable));
}

fn install_icon_refresh(image: &gtk::Image, state: Rc<std::cell::RefCell<Option<gio::Icon>>>) {
    let weak_image = image.downgrade();
    let scale_state = state.clone();
    image.connect_notify_local(Some("scale-factor"), move |_, _| {
        if let Some(image) = weak_image.upgrade() {
            refresh_file_list_icon(&image, &scale_state);
        }
    });

    let handler = Rc::new(std::cell::RefCell::new(None::<glib::SignalHandlerId>));
    let on_realize = {
        let weak_image = image.downgrade();
        let state = state.clone();
        let handler = handler.clone();
        move |_: &gtk::Image| {
            let Some(image) = weak_image.upgrade() else {
                return;
            };
            let theme = gtk::IconTheme::for_display(&image.display());
            if handler.borrow().is_none() {
                let weak_image = image.downgrade();
                let state = state.clone();
                let id = theme.connect_changed(move |_| {
                    if let Some(image) = weak_image.upgrade() {
                        refresh_file_list_icon(&image, &state);
                    }
                });
                *handler.borrow_mut() = Some(id);
            }
            refresh_file_list_icon(&image, &state);
        }
    };
    image.connect_realize(on_realize);
    let weak_image = image.downgrade();
    let unrealize_handler = handler.clone();
    image.connect_unrealize(move |_| {
        if let Some(id) = unrealize_handler.borrow_mut().take() {
            if let Some(image) = weak_image.upgrade() {
                gtk::IconTheme::for_display(&image.display()).disconnect(id);
            }
        }
    });
}

/// Icon + name row (used by compact and details).
fn name_factory(
    pixel_size: i32,
    on_secondary: &SecondaryHandler,
    on_middle: &MiddleHandler,
    on_drag_prepare: &DragPrepareHandler,
    on_drop: &DropHandler,
    on_external_move: &dnd::ExternalMoveHandler,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        row.add_css_class("file-item");
        let image = gtk::Image::new();
        image.set_pixel_size(pixel_size);
        row.append(&image);
        row.append(
            &gtk::Label::builder()
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&row));
    });
    factory.connect_bind({
        let on_secondary = on_secondary.clone();
        let on_middle = on_middle.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        let on_drop = on_drop.clone();
        let on_external_move = on_external_move.clone();
        move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            let obj = obj_of(list_item);
            let row = list_item.child().and_downcast::<gtk::Box>().unwrap();
            let icon = row.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = row.last_child().and_downcast::<gtk::Label>().unwrap();
            invalidate_thumbnail_binding(&icon);
            label.set_text(&obj.name());
            match obj.icon() {
                Some(gicon) => set_file_list_icon(&icon, &gicon),
                None => set_file_list_icon(&icon, &fallback_icon(obj.is_dir())),
            }
            if thumbnail_eligible(&obj) {
                start_thumbnail(&icon, &obj, pixel_size);
            }
            set_secondary(&row, &obj, &on_secondary);
            set_middle(&row, &obj, &on_middle);
            let dragged = obj.clone();
            let on_drag_prepare = on_drag_prepare.clone();
            dnd::attach_row_drag_drop(
                &row,
                obj.uri(),
                obj.is_dir(),
                Rc::new(move || on_drag_prepare(&dragged)),
                on_drop.clone(),
                on_external_move.clone(),
            );
        }
    });
    factory.connect_unbind(|_, list_item| {
        if let Some(image) = list_item
            .downcast_ref::<gtk::ListItem>()
            .and_then(|item| item.child())
            .and_then(|row| row.first_child())
            .and_downcast::<gtk::Image>()
        {
            invalidate_thumbnail_binding(&image);
        }
    });
    factory
}

/// Label-only column (Size, Type in details).
#[derive(Clone, Copy)]
struct TextCellStyle {
    xalign: f32,
    dim: bool,
    tabular: bool,
}

fn text_factory(
    get: fn(&FileObject) -> String,
    style: TextCellStyle,
    on_secondary: &SecondaryHandler,
    on_middle: &MiddleHandler,
    on_drag_prepare: &DragPrepareHandler,
    on_drop: &DropHandler,
    on_external_move: &dnd::ExternalMoveHandler,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let label = gtk::Label::builder()
            // Fills the cell (so the menu also arrives by clicking the
            // space right of the text), text still on the left.
            .xalign(style.xalign)
            .hexpand(true)
            .margin_start(8)
            .margin_end(8)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("file-item");
        if style.dim {
            // GTK's standard subdued-label styling remains theme-aware and
            // keeps the selected-row foreground under the theme's control.
            label.add_css_class("dim-label");
        }
        if style.tabular {
            let attributes = gtk::pango::AttrList::new();
            attributes.insert(gtk::pango::AttrFontFeatures::new("tnum"));
            label.set_attributes(Some(&attributes));
        }
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&label));
    });
    let on_secondary = on_secondary.clone();
    let on_middle = on_middle.clone();
    let on_drag_prepare = on_drag_prepare.clone();
    let on_drop = on_drop.clone();
    let on_external_move = on_external_move.clone();
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = list_item.child().and_downcast::<gtk::Label>().unwrap();
        let obj = obj_of(list_item);
        label.set_text(&get(&obj));
        set_secondary(&label, &obj, &on_secondary);
        set_middle(&label, &obj, &on_middle);
        let dragged = obj.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        dnd::attach_row_drag_drop(
            &label,
            obj.uri(),
            obj.is_dir(),
            Rc::new(move || on_drag_prepare(&dragged)),
            on_drop.clone(),
            on_external_move.clone(),
        );
    });
    factory
}

fn build_compact(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    zoom: u8,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let column = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(
            name_icon_pixel_size(22, zoom),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    column.set_expand(true);
    view.append_column(&column);
    view
}

fn build_details(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    display: ViewDisplay,
    sort_order: Rc<std::cell::Cell<kito_core::SortOrder>>,
    on_sort: Rc<dyn Fn(kito_core::SortOrder)>,
    update_widths: Rc<dyn Fn(DetailColumnWidths) -> u64>,
    persist_widths: Rc<dyn Fn(DetailColumnWidths, u64)>,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection));
    let name = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("column-name")),
        Some(name_factory(
            name_icon_pixel_size(20, display.icon_zoom),
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    name.set_id(Some("name"));
    name.set_resizable(true);
    name.set_expand(true);
    name.set_fixed_width(display.detail_column_widths.name);
    view.append_column(&name);
    let size = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-size")),
        Some(text_factory(
            |o| human_size(o.size()),
            TextCellStyle {
                xalign: 1.0,
                dim: false,
                tabular: true,
            },
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    size.set_id(Some("size"));
    size.set_resizable(true);
    size.set_fixed_width(display.detail_column_widths.size);
    size.set_visible(display.show_size);
    view.append_column(&size);
    let kind = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-type")),
        Some(text_factory(
            |o| o.type_label(),
            TextCellStyle {
                xalign: 0.0,
                dim: false,
                tabular: false,
            },
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    kind.set_id(Some("type"));
    kind.set_resizable(true);
    kind.set_fixed_width(display.detail_column_widths.kind);
    kind.set_visible(display.show_type);
    view.append_column(&kind);
    let modified = gtk::ColumnViewColumn::new(
        Some(&crate::l10n::tr("props-modified")),
        Some(text_factory(
            |o| modified_label(o.modified()),
            TextCellStyle {
                xalign: 0.0,
                dim: true,
                tabular: false,
            },
            handlers.on_secondary,
            handlers.on_middle,
            handlers.on_drag_prepare,
            handlers.on_drop,
            handlers.on_external_move,
        )),
    );
    modified.set_id(Some("modified"));
    modified.set_resizable(true);
    modified.set_fixed_width(display.detail_column_widths.modified);
    modified.set_visible(display.show_modified);
    view.append_column(&modified);

    // GTK emits fixed-width notifications while dragging. Debounce writes so
    // the settings file is touched only after the pointer has settled.
    let pending = Rc::new(std::cell::RefCell::new(None::<glib::SourceId>));
    let widths = Rc::new(std::cell::Cell::new(display.detail_column_widths));
    for column in [&name, &size, &kind, &modified] {
        let pending = pending.clone();
        let widths = widths.clone();
        let update_widths = update_widths.clone();
        let persist_widths = persist_widths.clone();
        let name = name.clone();
        let size = size.clone();
        let kind = kind.clone();
        let modified = modified.clone();
        column.connect_fixed_width_notify(move |_| {
            let current = DetailColumnWidths {
                name: name.fixed_width(),
                size: size.fixed_width(),
                kind: kind.fixed_width(),
                modified: modified.fixed_width(),
            }
            .clamped();
            widths.set(current);
            let generation = update_widths(current);
            if let Some(timer) = pending.borrow_mut().take() {
                timer.remove();
            }
            let pending_timer = pending.clone();
            let persist_widths = persist_widths.clone();
            let widths = widths.clone();
            let source =
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                    pending_timer.borrow_mut().take();
                    persist_widths(widths.get(), generation);
                });
            *pending.borrow_mut() = Some(source);
        });
    }

    // ColumnView's own headers are the clickable, localized column titles.
    // The view-level `header_factory` is for GtkListHeader section rows, not
    // column titles; using it here passes a non-ListItem to its callbacks.
    // Give each column a sorter so GTK enables the native header click and
    // arrow indicator. The app sorts its ListStore asynchronously via
    // `on_sort`, rather than installing GTK's sorter on a SortListModel.
    let click_sorter = gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal);
    name.set_sorter(Some(&click_sorter));
    size.set_sorter(Some(&click_sorter));
    kind.set_sorter(Some(&click_sorter));
    modified.set_sorter(Some(&click_sorter));

    let initial = sort_order.get();
    let initial_column = match initial.field {
        kito_core::SortField::Name => &name,
        kito_core::SortField::Size => &size,
        kito_core::SortField::Type => &kind,
        kito_core::SortField::Modified => &modified,
    };
    view.sort_by_column(Some(initial_column), gtk_sort_type(initial.direction));

    if let Some(sorter) = view.sorter().and_downcast::<gtk::ColumnViewSorter>() {
        sorter.connect_changed({
            let sort_order = sort_order.clone();
            move |sorter, _| {
                let Some(column) = sorter.primary_sort_column() else {
                    return;
                };
                let Some(field) = column.id().as_deref().and_then(sort_field_for_column) else {
                    return;
                };
                let direction = if sorter.primary_sort_order() == gtk::SortType::Descending {
                    kito_core::SortDirection::Descending
                } else {
                    kito_core::SortDirection::Ascending
                };
                let next = kito_core::SortOrder { field, direction };
                // Let the owner update this shared state and kick off its
                // asynchronous re-sort; setting it first would make that
                // callback treat the requested order as already applied.
                if sort_order.get() != next {
                    on_sort(next);
                }
            }
        });
    }
    view
}

fn gtk_sort_type(direction: kito_core::SortDirection) -> gtk::SortType {
    match direction {
        kito_core::SortDirection::Ascending => gtk::SortType::Ascending,
        kito_core::SortDirection::Descending => gtk::SortType::Descending,
    }
}

fn sort_field_for_column(id: &str) -> Option<kito_core::SortField> {
    match id {
        "name" => Some(kito_core::SortField::Name),
        "size" => Some(kito_core::SortField::Size),
        "type" => Some(kito_core::SortField::Type),
        "modified" => Some(kito_core::SortField::Modified),
        _ => None,
    }
}

fn modified_label(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|timestamp| glib::DateTime::from_unix_local(timestamp).ok())
        .and_then(|date| date.format("%x %H:%M").ok())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn build_icons(
    selection: gtk::MultiSelection,
    handlers: &ViewHandlers<'_>,
    zoom: u8,
) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, list_item| {
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_start(6)
            .margin_end(6)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        cell.set_size_request(icon_cell_width(zoom), -1);
        cell.add_css_class("file-item");
        let image = gtk::Image::new();
        image.set_pixel_size(icon_pixel_size(zoom));
        cell.append(&image);
        cell.append(
            &gtk::Label::builder()
                .halign(gtk::Align::Center)
                .justify(gtk::Justification::Center)
                // Fixed width in characters over 3 lines: the container
                // must never grow wider.
                .width_chars(12)
                .max_width_chars(12)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(3)
                .single_line_mode(false)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        list_item
            .downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&cell));
    });
    let on_secondary = handlers.on_secondary.clone();
    let on_middle = handlers.on_middle.clone();
    let on_drag_prepare = handlers.on_drag_prepare.clone();
    let on_drop = handlers.on_drop.clone();
    let on_external_move = handlers.on_external_move.clone();
    factory.connect_bind(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let obj = obj_of(list_item);
        let cell = list_item.child().and_downcast::<gtk::Box>().unwrap();
        let image = cell.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = cell.last_child().and_downcast::<gtk::Label>().unwrap();
        invalidate_thumbnail_binding(&image);
        label.set_text(&obj.name());
        match obj.icon() {
            Some(gicon) => image.set_from_gicon(&gicon),
            None => image.set_from_gicon(&fallback_icon(obj.is_dir())),
        }
        if thumbnail_eligible(&obj) {
            start_thumbnail(&image, &obj, icon_pixel_size(zoom));
        }
        set_secondary(&cell, &obj, &on_secondary);
        set_middle(&cell, &obj, &on_middle);
        let dragged = obj.clone();
        let on_drag_prepare = on_drag_prepare.clone();
        dnd::attach_row_drag_drop(
            &cell,
            obj.uri(),
            obj.is_dir(),
            Rc::new(move || on_drag_prepare(&dragged)),
            on_drop.clone(),
            on_external_move.clone(),
        );
    });
    factory.connect_unbind(|_, list_item| {
        if let Some(image) = list_item
            .downcast_ref::<gtk::ListItem>()
            .and_then(|item| item.child())
            .and_then(|cell| cell.first_child())
            .and_downcast::<gtk::Image>()
        {
            invalidate_thumbnail_binding(&image);
        }
    });
    gtk::GridView::new(Some(selection), Some(factory))
}

/// Builds the view for `mode` on the given store. Returns widget and selection.
#[allow(clippy::too_many_arguments)]
pub fn build_view(
    mode: ViewMode,
    store: &gio::ListStore,
    handlers: &ViewHandlers<'_>,
    open_items: OpenItems,
    display: ViewDisplay,
    sort_order: Rc<std::cell::Cell<kito_core::SortOrder>>,
    on_sort: Rc<dyn Fn(kito_core::SortOrder)>,
    update_widths: Rc<dyn Fn(DetailColumnWidths) -> u64>,
    persist_widths: Rc<dyn Fn(DetailColumnWidths, u64)>,
) -> (gtk::Widget, gtk::MultiSelection) {
    let selection = gtk::MultiSelection::new(Some(store.clone()));
    let widget: gtk::Widget = match mode {
        ViewMode::Icons => build_icons(selection.clone(), handlers, display.icon_zoom).upcast(),
        ViewMode::Compact => build_compact(selection.clone(), handlers, display.icon_zoom).upcast(),
        ViewMode::Details => build_details(
            selection.clone(),
            handlers,
            display,
            sort_order,
            on_sort,
            update_widths,
            persist_widths,
        )
        .upcast(),
    };
    // Activation (pointer or keyboard) has one shared handler in FileTab.
    set_open_items(&widget, open_items);
    widget.set_vexpand(true);
    (widget, selection)
}

/// Attaches file-view-local shortcuts so text entries and dialogs keep their
/// normal keyboard behavior.
pub fn add_file_shortcuts(widget: &gtk::Widget) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Local);
    for (accelerator, name) in crate::shortcuts::FILE_VIEW_SHORTCUTS {
        if let (Some(trigger), Some(action)) = (
            gtk::ShortcutTrigger::parse_string(accelerator),
            gtk::ShortcutAction::parse_string(name),
        ) {
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    }
    widget.add_controller(controller);
}

/// True while a Ctrl/Shift pointer activation should only change selection.
pub fn add_modified_click_guard(widget: &gtk::Widget) -> Rc<std::cell::Cell<bool>> {
    let suppress = Rc::new(std::cell::Cell::new(false));
    let gesture = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .build();
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = suppress.clone();
    gesture.connect_pressed(move |gesture, _, _, _| {
        let modifiers = gesture.current_event_state();
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
        {
            state.set(true);
            let state = state.clone();
            glib::idle_add_local_once(move || state.set(false));
        }
    });
    widget.add_controller(gesture);
    suppress
}

/// Configure GTK's built-in pointer activation while leaving Enter and the
/// view's single `activate` signal handler untouched.
pub fn set_open_items(widget: &gtk::Widget, behavior: OpenItems) {
    let single = single_click_activation(behavior);
    if let Ok(view) = widget.clone().downcast::<gtk::ColumnView>() {
        view.set_single_click_activate(single);
    } else if let Ok(view) = widget.clone().downcast::<gtk::GridView>() {
        view.set_single_click_activate(single);
    }
}

fn single_click_activation(behavior: OpenItems) -> bool {
    behavior == OpenItems::SingleClick
}

/// View + empty store; populate with [`reload`].
pub fn build_file_view() -> (gtk::ScrolledWindow, gio::ListStore) {
    let store = gio::ListStore::new::<FileObject>();
    let scrolled = gtk::ScrolledWindow::builder().vexpand(true).build();
    (scrolled, store)
}

/// Rows created and inserted per main-loop turn during asynchronous loads.
/// Keeps each pause short while staying near single-splice total time.
pub const LOAD_CHUNK: usize = 500;

/// Replaces the whole store content with `entries` in a single `splice`:
/// one model notification instead of one per row.
#[cfg(test)]
pub fn replace_all(store: &gio::ListStore, entries: &[kito_core::Entry]) {
    let objs: Vec<FileObject> = entries.iter().map(FileObject::new).collect();
    store.splice(0, store.n_items(), &objs);
}

/// Appends one chunk of entries with a single `splice` (one notification).
/// The caller clears the store first for a fresh load.
pub fn append_chunk(store: &gio::ListStore, chunk: &[kito_core::Entry]) {
    if chunk.is_empty() {
        return;
    }
    let objs: Vec<FileObject> = chunk.iter().map(FileObject::new).collect();
    store.splice(store.n_items(), 0, &objs);
}

/// Fills the store with `dir_uri`. Returns the entry count.
/// Test helper: the UI loads asynchronously (see `tabs`), sharing the same
/// `list_dir` ordering and single-splice application.
#[cfg(test)]
pub fn reload(
    store: &gio::ListStore,
    dir_uri: &str,
    show_hidden: bool,
) -> Result<usize, glib::Error> {
    let entries = kito_core::list_dir(dir_uri, show_hidden)?;
    replace_all(store, &entries);
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_entry(uri: String, size: i64, mime: &str) -> FileObject {
        FileObject::new(&kito_core::Entry {
            name: "fixture.png".into(),
            uri,
            is_dir: false,
            size,
            modified: Some(1),
            content_type: Some(mime.into()),
            icon: None,
        })
    }

    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn thumbnails_require_local_regular_image_mime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        std::fs::write(&path, PNG).unwrap();
        let uri = gio::File::for_path(&path).uri().to_string();
        assert!(thumbnail_eligible(&image_entry(
            uri.clone(),
            PNG.len() as i64,
            "image/png"
        )));
        assert!(!thumbnail_eligible(&image_entry(
            uri.clone(),
            PNG.len() as i64,
            "text/plain"
        )));
        assert!(!thumbnail_eligible(&image_entry(
            "https://example.invalid/x.png".into(),
            1,
            "image/png"
        )));
        assert!(!thumbnail_eligible(&image_entry(
            uri,
            (THUMB_MAX_INPUT + 1) as i64,
            "image/png"
        )));
    }

    #[test]
    fn thumbnail_decode_is_bounded_and_bad_inputs_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        std::fs::write(&path, PNG).unwrap();
        let uri = gio::File::for_path(&path).uri().to_string();
        let thumb = decode_thumbnail(uri.clone(), 32).unwrap();
        assert_eq!((thumb.width, thumb.height), (1, 1));
        assert!(thumb.data.len() <= 4);
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode_thumbnail(uri.clone(), 32).is_none());
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(THUMB_MAX_INPUT + 1).unwrap();
        assert!(decode_thumbnail(uri, 32).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn thumbnail_decode_rejects_fifo_without_writer() {
        use std::os::unix::ffi::OsStrExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.fifo");
        let path_c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // No external command or writer: a blocking open would hang this test.
        assert_eq!(unsafe { libc::mkfifo(path_c.as_ptr(), 0o600) }, 0);
        let uri = gio::File::for_path(path).uri().to_string();
        assert!(decode_thumbnail(uri, 32).is_none());
    }

    #[test]
    fn recycled_thumbnail_binding_rejects_stale_uri_or_generation() {
        let binding = ThumbnailBinding::default();
        binding.generation.set(8);
        *binding.uri.borrow_mut() = "file:///new".into();
        assert!(current_binding(&binding, 8, "file:///new"));
        assert!(!current_binding(&binding, 7, "file:///new"));
        assert!(!current_binding(&binding, 8, "file:///old"));
    }

    #[test]
    fn thumbnail_queue_services_fifo_with_four_active_dedups_and_skips_stale() {
        #[derive(Clone)]
        struct Request {
            widget: usize,
            id: usize,
            valid: bool,
            uri: String,
        }
        impl ThumbnailQueueTarget for Request {
            fn same_widget(&self, other: &Self) -> bool {
                self.widget == other.widget
            }
            fn is_live(&self) -> bool {
                self.valid
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut queue = ThumbnailQueue::default();
        for id in 0..8 {
            let path = dir.path().join(format!("{id}.png"));
            std::fs::write(&path, PNG).unwrap();
            assert!(queue.enqueue(
                format!("key-{id}"),
                Request {
                    widget: id,
                    id,
                    valid: true,
                    uri: gio::File::for_path(path).uri().to_string(),
                }
            ));
        }
        // Same key is coalesced into one queued decode with both consumers.
        assert!(queue.enqueue(
            "key-3".into(),
            Request {
                widget: 30,
                id: 30,
                valid: true,
                uri: gio::File::for_path(dir.path().join("3.png"))
                    .uri()
                    .to_string(),
            }
        ));
        // This request becomes stale before its turn in the FIFO.
        assert!(queue.enqueue(
            "stale".into(),
            Request {
                widget: 99,
                id: 99,
                valid: false,
                uri: gio::File::for_path(dir.path().join("stale.png"))
                    .uri()
                    .to_string(),
            }
        ));
        std::fs::write(dir.path().join("stale.png"), PNG).unwrap();

        let mut active = Vec::new();
        while let Some((key, first)) = queue.start_next() {
            active.push(key);
            assert_eq!(queue.active, active.len());
            assert!(queue.active <= THUMB_MAX_ACTIVE);
            assert_eq!(first.id, active.len() - 1);
        }
        assert_eq!(active.len(), 4);

        let mut completed = Vec::new();
        while !active.is_empty() || !queue.order.is_empty() {
            if let Some(key) = active.pop() {
                let targets = queue.finish(&key);
                let current: Vec<_> = targets.into_iter().filter(|target| target.valid).collect();
                if let Some(target) = current.first() {
                    assert!(decode_thumbnail(target.uri.clone(), 24).is_some());
                    completed.extend(current.into_iter().map(|target| target.id));
                }
            }
            while let Some((key, _)) = queue.start_next() {
                let targets = queue.targets_mut(&key).unwrap();
                targets.retain(|target| target.valid);
                if targets.is_empty() {
                    queue.finish(&key);
                } else {
                    active.push(key);
                }
                assert!(queue.active <= THUMB_MAX_ACTIVE);
            }
        }
        assert!((0..8).all(|id| completed.contains(&id)));
        assert!(completed.contains(&30));
        assert!(!completed.contains(&99));
        assert_eq!(queue.active, 0);
    }

    #[test]
    fn thumbnail_queue_bounds_unique_waiting_jobs() {
        #[derive(Clone)]
        struct Request(usize);
        impl ThumbnailQueueTarget for Request {
            fn same_widget(&self, other: &Self) -> bool {
                self.0 == other.0
            }
            fn is_live(&self) -> bool {
                true
            }
        }
        let mut queue = ThumbnailQueue::default();
        for id in 0..THUMB_MAX_ACTIVE {
            assert!(queue.enqueue(format!("active-{id}"), Request(id)));
        }
        for _ in 0..THUMB_MAX_ACTIVE {
            queue.start_next().unwrap();
        }
        for id in 0..THUMB_MAX_WAITING {
            assert!(queue.enqueue(format!("waiting-{id}"), Request(id + THUMB_MAX_ACTIVE)));
        }
        assert_eq!(queue.waiting.len(), THUMB_MAX_WAITING);
        assert!(!queue.enqueue("overflow".into(), Request(usize::MAX)));
        assert_eq!(queue.waiting.len(), THUMB_MAX_WAITING);
    }

    #[test]
    fn thumbnail_queue_rebind_replaces_widget_target_across_keys() {
        #[derive(Clone)]
        struct Request {
            widget: usize,
            generation: usize,
        }
        impl ThumbnailQueueTarget for Request {
            fn same_widget(&self, other: &Self) -> bool {
                self.widget == other.widget
            }
            fn is_live(&self) -> bool {
                true
            }
        }

        let mut queue = ThumbnailQueue::default();
        for generation in 0..1000 {
            assert!(queue.enqueue(
                "same".into(),
                Request {
                    widget: 1,
                    generation
                }
            ));
            assert_eq!(queue.waiting["same"].len(), 1);
        }
        assert_eq!(queue.waiting["same"][0].generation, 999);

        queue.enqueue(
            "shared".into(),
            Request {
                widget: 2,
                generation: 1,
            },
        );
        queue.enqueue(
            "shared".into(),
            Request {
                widget: 3,
                generation: 1,
            },
        );
        queue.enqueue(
            "new-key".into(),
            Request {
                widget: 1,
                generation: 1000,
            },
        );
        assert!(!queue.waiting.contains_key("same"));
        assert_eq!(queue.waiting["new-key"].len(), 1);
        assert_eq!(queue.waiting["shared"].len(), 2);
        assert_eq!(queue.waiting["new-key"][0].generation, 1000);
    }

    #[test]
    fn thumbnail_queue_repeatedly_prunes_dead_targets_across_all_keys() {
        #[derive(Clone)]
        struct Request {
            widget: usize,
            alive: bool,
        }
        impl ThumbnailQueueTarget for Request {
            fn same_widget(&self, other: &Self) -> bool {
                self.widget == other.widget
            }
            fn is_live(&self) -> bool {
                self.alive
            }
        }

        let mut queue = ThumbnailQueue::default();
        assert!(queue.enqueue(
            "running".into(),
            Request {
                widget: 1,
                alive: true
            }
        ));
        queue.start_next().unwrap();
        // Keep a valid sibling attached to the running shared job.
        assert!(queue.enqueue(
            "running".into(),
            Request {
                widget: 2,
                alive: true
            }
        ));
        assert!(queue.enqueue(
            "dead-waiting".into(),
            Request {
                widget: 3,
                alive: false
            }
        ));
        assert_eq!(queue.waiting.len(), 1);
        for id in 0..1000 {
            assert!(queue.enqueue(
                "fresh".into(),
                Request {
                    widget: id + 10,
                    alive: true
                }
            ));
            assert!(!queue.waiting.contains_key("dead-waiting"));
            assert!(!queue.order.iter().any(|key| key == "dead-waiting"));
            assert_eq!(queue.running["running"].len(), 2);
            // Newly introduced dead targets in arbitrary buckets are pruned on
            // the next request, never accumulating with rebuilds.
            assert!(queue.enqueue(
                "dead".into(),
                Request {
                    widget: id + 2000,
                    alive: false
                }
            ));
            assert!(queue.enqueue(
                "cleanup".into(),
                Request {
                    widget: id + 4000,
                    alive: true
                }
            ));
            assert!(!queue.waiting.contains_key("dead"));
            assert_eq!(queue.waiting.len(), 2);
        }
        assert_eq!(queue.finish("running").len(), 2);
    }

    #[test]
    fn thumbnail_cache_obeys_entry_and_byte_budgets() {
        let pixels = |size: usize| {
            Some(ThumbnailPixels {
                data: glib::Bytes::from_owned(vec![0; size]),
                width: 1,
                height: 1,
                rowstride: 4,
                alpha: true,
            })
        };
        let mut cache = std::collections::VecDeque::new();
        cache.push_front(("negative".into(), None));
        cache.push_front(("old-large".into(), pixels(THUMB_CACHE_BYTE_LIMIT / 2 + 1)));
        cache.push_front(("new-large".into(), pixels(THUMB_CACHE_BYTE_LIMIT / 2 + 1)));
        trim_thumbnail_cache(&mut cache);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].0, "new-large");
        assert_eq!(
            cache
                .iter()
                .map(|(_, p)| p.as_ref().map_or(0, |p| p.data.len()))
                .sum::<usize>(),
            THUMB_CACHE_BYTE_LIMIT / 2 + 1
        );

        let mut cache = std::collections::VecDeque::new();
        for id in 0..=THUMB_CACHE_LIMIT {
            cache.push_front((id.to_string(), None));
        }
        trim_thumbnail_cache(&mut cache);
        assert_eq!(cache.len(), THUMB_CACHE_LIMIT);
        assert_eq!(cache.front().unwrap().0, THUMB_CACHE_LIMIT.to_string());
    }

    #[test]
    fn open_setting_only_switches_gtk_pointer_activation() {
        assert!(!single_click_activation(OpenItems::DoubleClick));
        assert!(single_click_activation(OpenItems::SingleClick));
        // wire_activate remains the only activation-signal hookup for both
        // view types; changing this flag never attaches another handler.
    }

    #[test]
    fn icon_zoom_matches_the_larger_grid_reference() {
        assert_eq!(icon_pixel_size(60), 57);
        assert_eq!(icon_pixel_size(100), 96);
        assert_eq!(icon_pixel_size(180), 172);
        assert_eq!(icon_cell_width(60), 96);
        assert_eq!(icon_cell_width(100), 160);
        assert_eq!(icon_cell_width(180), 288);
    }

    #[test]
    fn name_view_icons_keep_theme_asset_size_floor_and_follow_zoom() {
        for base_size in [22, 20] {
            // Compact and Details
            assert_eq!(name_icon_pixel_size(base_size, 100), 26);
            assert!(name_icon_pixel_size(base_size, 150) > 26);
            assert!(name_icon_pixel_size(base_size, 200) > name_icon_pixel_size(base_size, 150));
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn file_list_icon_lookup_uses_regular_theme_paintable() {
        gtk::test_synced(|| {
            let image = gtk::Image::new();
            image.set_pixel_size(name_icon_pixel_size(20, 100));
            let icon = gio::ThemedIcon::from_names(&["folder-documents", "folder"]);
            set_file_list_icon(&image, &icon);

            let paintable = image
                .paintable()
                .and_downcast::<gtk::IconPaintable>()
                .expect("theme lookup should install an icon paintable");
            let resolved_file = paintable
                .file()
                .and_then(|file| file.path())
                .expect("theme icon should resolve to an asset");
            assert!(resolved_file.exists(), "resolved asset: {resolved_file:?}");
            assert!(
                !resolved_file
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.ends_with("-symbolic")),
                "file-list icons must resolve to regular theme assets: {resolved_file:?}"
            );
        });
    }

    fn listed_uris(store: &gio::ListStore) -> Vec<String> {
        (0..store.n_items())
            .filter_map(|i| {
                store
                    .item(i)
                    .and_downcast::<FileObject>()
                    .map(|obj| obj.uri())
            })
            .collect()
    }

    fn fixture_entries(n: usize) -> Vec<kito_core::Entry> {
        (0..n)
            .map(|i| kito_core::Entry {
                name: format!("file-{i:05}"),
                uri: format!("file:///tmp/file-{i:05}"),
                is_dir: i % 7 == 0,
                size: i as i64,
                modified: Some(i as i64),
                content_type: None,
                icon: None,
            })
            .collect()
    }

    fn widget_contains_label(widget: &gtk::Widget, expected: &str) -> bool {
        if widget
            .clone()
            .downcast::<gtk::Label>()
            .is_ok_and(|label| label.text() == expected)
        {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if widget_contains_label(&current, expected) {
                return true;
            }
            child = current.next_sibling();
        }
        false
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn details_cells_bind_and_native_column_headers_sort_both_directions() {
        gtk::test_synced(|| {
            let store = gio::ListStore::new::<FileObject>();
            replace_all(&store, &fixture_entries(3));
            let on_secondary: SecondaryHandler = Rc::new(|_, _, _, _| {});
            let on_middle: MiddleHandler = Rc::new(|_| {});
            let on_drag_prepare: DragPrepareHandler = Rc::new(|_| None);
            let on_drop: DropHandler = Rc::new(|_, _, _, _, _| {});
            let on_external_move: dnd::ExternalMoveHandler = Rc::new(|_| {});
            let handlers = ViewHandlers {
                on_secondary: &on_secondary,
                on_middle: &on_middle,
                on_drag_prepare: &on_drag_prepare,
                on_drop: &on_drop,
                on_external_move: &on_external_move,
            };
            let display = ViewDisplay {
                icon_zoom: 100,
                show_size: false,
                show_type: true,
                show_modified: false,
                detail_column_widths: DetailColumnWidths {
                    name: 330,
                    size: 90,
                    kind: 250,
                    modified: 190,
                },
            };
            let sort_order = Rc::new(std::cell::Cell::new(kito_core::SortOrder::default()));
            let observed_orders = Rc::new(std::cell::RefCell::new(Vec::new()));
            let on_sort: Rc<dyn Fn(kito_core::SortOrder)> = Rc::new({
                let sort_order = sort_order.clone();
                let observed_orders = observed_orders.clone();
                move |order| {
                    sort_order.set(order);
                    observed_orders.borrow_mut().push(order);
                }
            });

            let (widget, selection) = build_view(
                ViewMode::Details,
                &store,
                &handlers,
                OpenItems::DoubleClick,
                display,
                sort_order,
                on_sort,
                Rc::new(|_| 0),
                Rc::new(|_, _| {}),
            );
            let view = widget.clone().downcast::<gtk::ColumnView>().unwrap();
            // ColumnView.header_factory configures section headers, not the
            // column titles. GTK's real ColumnViewCells still run the row factory.
            assert!(view.header_factory().is_none());

            let columns = view.columns();
            let column = |id: &str| {
                (0..columns.n_items())
                    .filter_map(|index| columns.item(index).and_downcast::<gtk::ColumnViewColumn>())
                    .find(|column| column.id().as_deref() == Some(id))
                    .unwrap()
            };
            let name = column("name");
            let size = column("size");
            let kind = column("type");
            let modified = column("modified");
            assert!(name.sorter().is_some());
            assert!(size.sorter().is_some());
            assert!(kind.sorter().is_some());
            assert!(modified.sorter().is_some());
            assert!(
                name.is_resizable()
                    && size.is_resizable()
                    && kind.is_resizable()
                    && modified.is_resizable()
            );
            assert_eq!(name.fixed_width(), 330);
            assert_eq!(size.fixed_width(), 90);
            assert_eq!(kind.fixed_width(), 250);
            assert_eq!(modified.fixed_width(), 190);
            assert!(name.is_visible());
            assert!(!size.is_visible());
            assert!(kind.is_visible());
            assert!(!modified.is_visible());
            let localized_name = crate::l10n::tr("column-name");
            assert_eq!(name.title().as_deref(), Some(localized_name.as_str()));

            // Map the real ColumnView so GTK invokes the row factory's setup and
            // bind callbacks on its ColumnViewCell objects.
            let window = gtk::Window::new();
            window.set_default_size(640, 240);
            window.set_child(Some(&widget));
            window.present();
            let context = glib::MainContext::default();
            for _ in 0..10 {
                while context.pending() {
                    context.iteration(false);
                }
            }
            assert!(widget_contains_label(&widget, "file-00000"));

            selection.select_item(1, false);
            let view_sorter = view
                .sorter()
                .and_downcast::<gtk::ColumnViewSorter>()
                .unwrap();
            assert_eq!(
                view_sorter
                    .primary_sort_column()
                    .and_then(|column| column.id())
                    .as_deref(),
                Some("name")
            );
            assert_eq!(view_sorter.primary_sort_order(), gtk::SortType::Ascending);

            // `sort_by_column` is the same GTK sorter path used by clicking the
            // native column header. It verifies first-click ascending, repeated
            // clicks toggling direction, and the app's sort callback.
            view.sort_by_column(Some(&name), gtk::SortType::Descending);
            view.sort_by_column(Some(&name), gtk::SortType::Ascending);
            view.sort_by_column(Some(&size), gtk::SortType::Ascending);
            view.sort_by_column(Some(&size), gtk::SortType::Descending);
            assert_eq!(
                *observed_orders.borrow(),
                vec![
                    kito_core::SortOrder {
                        field: kito_core::SortField::Name,
                        direction: kito_core::SortDirection::Descending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Name,
                        direction: kito_core::SortDirection::Ascending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Size,
                        direction: kito_core::SortDirection::Ascending,
                    },
                    kito_core::SortOrder {
                        field: kito_core::SortField::Size,
                        direction: kito_core::SortDirection::Descending,
                    },
                ]
            );
            assert!(selection.is_selected(1));
            assert_eq!(view_sorter.primary_sort_order(), gtk::SortType::Descending);
            window.close();
        });
    }

    #[test]
    fn replace_all_fills_in_order_with_one_notification() {
        let store = gio::ListStore::new::<FileObject>();
        let emissions = std::rc::Rc::new(std::cell::Cell::new(0u32));
        store.connect_items_changed({
            let emissions = emissions.clone();
            move |_, _, _, _| emissions.set(emissions.get() + 1)
        });
        let entries = fixture_entries(1000);
        replace_all(&store, &entries);
        assert_eq!(store.n_items(), 1000);
        assert_eq!(listed_uris(&store).len(), 1000);
        assert_eq!(listed_uris(&store)[7], "file:///tmp/file-00007");
        // Bulk: a single model notification, not one per row.
        assert_eq!(emissions.get(), 1);

        // Refill replaces without growing: same count, new content, still one shot.
        emissions.set(0);
        let mut again = fixture_entries(1000);
        again[0].uri = "file:///tmp/changed".to_string();
        replace_all(&store, &again);
        assert_eq!(store.n_items(), 1000);
        assert_eq!(listed_uris(&store)[0], "file:///tmp/changed");
        assert_eq!(emissions.get(), 1);
    }

    #[test]
    fn chunked_append_matches_replace_all() {
        let entries = fixture_entries(1234);
        let full = gio::ListStore::new::<FileObject>();
        replace_all(&full, &entries);

        let chunked = gio::ListStore::new::<FileObject>();
        chunked.remove_all();
        for chunk in entries.chunks(LOAD_CHUNK) {
            append_chunk(&chunked, chunk);
        }
        assert_eq!(chunked.n_items(), full.n_items());
        assert_eq!(listed_uris(&chunked), listed_uris(&full));
    }

    #[test]
    fn append_chunk_empty_is_noop() {
        let store = gio::ListStore::new::<FileObject>();
        let emissions = std::rc::Rc::new(std::cell::Cell::new(0u32));
        store.connect_items_changed({
            let emissions = emissions.clone();
            move |_, _, _, _| emissions.set(emissions.get() + 1)
        });
        append_chunk(&store, &[]);
        assert_eq!(store.n_items(), 0);
        assert_eq!(emissions.get(), 0);
    }
}
