use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};

use eframe::egui::{self, Context, Response, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::markdown;

pub(crate) const MAX_GENERATED_SVG_CACHE_ENTRIES: usize = 128;
pub(crate) const MAX_GENERATED_SVG_CACHE_BYTES: usize = 32 * 1024 * 1024;

const LOCAL_IMAGE_CHECK_INTERVAL: Duration = Duration::from_millis(500);
// Metadata checks stay cheap. Equal metadata gets a slower content check so
// timestamp-preserving writes still refresh cached pixels.
const LOCAL_IMAGE_CONTENT_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const LOCAL_IMAGE_CONTENT_CHECK_BYTES_PER_SECOND: u64 = 8 * 1024 * 1024;
const LOCAL_IMAGE_SYNCHRONOUS_HASH_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ResolvedLocalImage {
    pub(crate) uri: Result<String, String>,
    pub(crate) revision: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ImageFileStamp {
    length: u64,
    modified: Option<SystemTime>,
}

struct LocalImageEntry {
    stamp: Option<ImageFileStamp>,
    // Outer None means the first background hash has not completed; inner
    // None means the file could not be read.
    content_hash: Option<Option<u64>>,
    pending_content_hash: Option<mpsc::Receiver<Option<u64>>>,
    content_checked_at: Instant,
    resolved: ResolvedLocalImage,
    checked_at: Instant,
    owners: HashSet<(u64, markdown::BlockId)>,
    context: Context,
}

/// Owns local-resource freshness. URI syntax stays compatible with egui's file
/// loader; an observed file change invalidates every egui image cache layer.
pub(crate) struct LocalImageStore {
    entries: HashMap<PathBuf, LocalImageEntry>,
    owners: HashMap<(u64, markdown::BlockId), PathBuf>,
    check_interval: Duration,
    next_revision: u64,
}

impl Default for LocalImageStore {
    fn default() -> Self {
        Self::new(LOCAL_IMAGE_CHECK_INTERVAL)
    }
}

impl LocalImageStore {
    pub(crate) fn new(check_interval: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            owners: HashMap::new(),
            check_interval,
            next_revision: 0,
        }
    }

    pub(crate) fn resolve(
        &mut self,
        ctx: &Context,
        owner: (u64, markdown::BlockId),
        base_directory: &Path,
        destination: &str,
    ) -> ResolvedLocalImage {
        let path = match local_image_path(base_directory, destination) {
            Ok(path) => std::path::absolute(&path).unwrap_or(path),
            Err(error) => {
                self.forget_block(owner);
                return ResolvedLocalImage {
                    uri: Err(error),
                    revision: 0,
                };
            }
        };
        if self.owners.get(&owner) != Some(&path) {
            self.forget_block(owner);
            self.owners.insert(owner, path.clone());
        }
        let now = Instant::now();
        if !self.check_interval.is_zero() {
            ctx.request_repaint_after(self.check_interval);
        }
        if let Some(entry) = self.entries.get_mut(&path) {
            entry.owners.insert(owner);
            if now.duration_since(entry.checked_at) < self.check_interval {
                return entry.resolved.clone();
            }
        }

        let stamp = std::fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| ImageFileStamp {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            });
        let uri = image_uri_for_path(&path);
        let mut checked_hash = None;
        if let Some(entry) = self.entries.get_mut(&path)
            && entry.stamp == stamp
            && entry.resolved.uri == uri
        {
            if let Some(pending) = entry.pending_content_hash.as_ref() {
                match pending.try_recv() {
                    Ok(hash) => {
                        entry.pending_content_hash = None;
                        checked_hash = Some(hash);
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        entry.checked_at = now;
                        return entry.resolved.clone();
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        entry.pending_content_hash = None;
                    }
                }
            }
            let content_check_interval = if self.check_interval.is_zero() {
                Duration::ZERO
            } else {
                let size_interval = stamp.as_ref().map_or(Duration::ZERO, |stamp| {
                    Duration::from_secs(
                        stamp
                            .length
                            .div_ceil(LOCAL_IMAGE_CONTENT_CHECK_BYTES_PER_SECOND),
                    )
                });
                LOCAL_IMAGE_CONTENT_CHECK_INTERVAL.max(size_interval)
            };
            if checked_hash.is_none()
                && now.duration_since(entry.content_checked_at) < content_check_interval
            {
                entry.checked_at = now;
                return entry.resolved.clone();
            }
            if checked_hash.is_none() {
                if stamp
                    .as_ref()
                    .is_some_and(|stamp| stamp.length > LOCAL_IMAGE_SYNCHRONOUS_HASH_BYTES)
                {
                    entry.pending_content_hash = spawn_local_image_content_hash(&path, ctx);
                    entry.content_checked_at = now;
                    entry.checked_at = now;
                    return entry.resolved.clone();
                }
                checked_hash = Some(local_image_content_hash(&path));
            }
            if entry.content_hash == checked_hash {
                entry.content_checked_at = now;
                entry.checked_at = now;
                return entry.resolved.clone();
            }
        }

        if let Some(entry) = self.entries.get(&path)
            && let Ok(previous_uri) = &entry.resolved.uri
        {
            ctx.forget_image(previous_uri);
        }
        // The same canonical file may have been loaded through another spelling
        // of its path, or before this store first observed it.
        if let Ok(uri) = &uri {
            ctx.forget_image(uri);
        }
        self.next_revision = self.next_revision.wrapping_add(1);
        let resolved = ResolvedLocalImage {
            uri,
            revision: self.next_revision,
        };
        let mut owners = self
            .entries
            .remove(&path)
            .map_or_else(HashSet::new, |entry| entry.owners);
        owners.insert(owner);
        let (content_hash, pending_content_hash) = if let Some(hash) = checked_hash {
            (Some(hash), None)
        } else if stamp
            .as_ref()
            .is_some_and(|stamp| stamp.length > LOCAL_IMAGE_SYNCHRONOUS_HASH_BYTES)
        {
            (None, spawn_local_image_content_hash(&path, ctx))
        } else {
            (Some(local_image_content_hash(&path)), None)
        };
        self.entries.insert(
            path,
            LocalImageEntry {
                stamp,
                content_hash,
                pending_content_hash,
                content_checked_at: now,
                resolved: resolved.clone(),
                checked_at: now,
                owners,
                context: ctx.clone(),
            },
        );
        resolved
    }

    pub(crate) fn forget_block(&mut self, owner: (u64, markdown::BlockId)) {
        let Some(path) = self.owners.remove(&owner) else {
            return;
        };
        let remove = self.entries.get_mut(&path).is_some_and(|entry| {
            entry.owners.remove(&owner);
            entry.owners.is_empty()
        });
        if remove
            && let Some(entry) = self.entries.remove(&path)
            && let Ok(uri) = entry.resolved.uri
        {
            entry.context.forget_image(&uri);
        }
    }

    pub(crate) fn forget_document(&mut self, document_id: u64) {
        let owners = self
            .owners
            .keys()
            .copied()
            .filter(|(owner_document, _)| *owner_document == document_id)
            .collect::<Vec<_>>();
        for owner in owners {
            self.forget_block(owner);
        }
    }
}

fn local_image_content_hash(path: &Path) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hasher.finish());
        }
        hasher.write(&buffer[..read]);
    }
}

fn spawn_local_image_content_hash(
    path: &Path,
    ctx: &Context,
) -> Option<mpsc::Receiver<Option<u64>>> {
    let (sender, receiver) = mpsc::channel();
    let path = path.to_owned();
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("rupora-image-hash".to_owned())
        .spawn(move || {
            let _ = sender.send(local_image_content_hash(&path));
            ctx.request_repaint();
        })
        .ok()?;
    Some(receiver)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeImage {
    pub range: std::ops::Range<usize>,
    pub destination: String,
    pub alt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeMath {
    pub range: std::ops::Range<usize>,
    pub source: String,
}

#[cfg(test)]
pub(crate) fn standalone_image(source: &str) -> Option<NativeImage> {
    standalone_image_with_references(source, &Default::default())
}

pub(crate) fn standalone_image_with_references(
    source: &str,
    references: &markdown::ReferenceDefinitions,
) -> Option<NativeImage> {
    let mut images = Vec::new();
    let mut active = None;
    for (event, range) in markdown::events_with_references(source, references) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                active = Some(NativeImage {
                    range,
                    destination: dest_url.into_string(),
                    alt: String::new(),
                });
            }
            Event::Text(text) | Event::Code(text) if active.is_some() => {
                if let Some(image) = active.as_mut() {
                    image.alt.push_str(&text);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(image) = active.take() {
                    images.push(image);
                }
            }
            _ => {}
        }
    }
    let image = images.pop()?;
    images.is_empty().then_some(())?;
    source[..image.range.start]
        .trim()
        .is_empty()
        .then_some(())?;
    source[image.range.end..].trim().is_empty().then_some(image)
}

pub(crate) fn standalone_display_math(source: &str) -> Option<NativeMath> {
    let mut math = None;
    for (event, range) in Parser::new_ext(source, markdown::parser_options()).into_offset_iter() {
        match event {
            Event::DisplayMath(value) if math.is_none() => {
                math = Some(NativeMath {
                    range,
                    source: value.into_string(),
                });
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            Event::Text(value) if value.trim().is_empty() => {}
            Event::SoftBreak | Event::HardBreak => {}
            Event::DisplayMath(_) => return None,
            _ => return None,
        }
    }
    math
}

pub(crate) fn standalone_mermaid(source: &str) -> Option<markdown::MermaidBlock> {
    let mut blocks = markdown::mermaid_blocks(source);
    let block = blocks.pop()?;
    blocks.is_empty().then_some(())?;
    source[..block.range.start]
        .trim()
        .is_empty()
        .then_some(())?;
    source[block.range.end..].trim().is_empty().then_some(block)
}

#[cfg(test)]
pub(crate) fn document_image_uri(
    base_directory: &Path,
    destination: &str,
) -> Result<String, String> {
    image_uri_for_path(&local_image_path(base_directory, destination)?)
}

fn local_image_path(base_directory: &Path, destination: &str) -> Result<PathBuf, String> {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    if path_part.is_empty() {
        return Err("图片路径为空".to_owned());
    }
    if has_uri_scheme(path_part) && !Path::new(path_part).is_absolute() {
        return Err("远程图片不会自动联网加载".to_owned());
    }
    let decoded =
        decode_local_resource_path(destination).ok_or_else(|| "图片路径编码无效".to_owned())?;
    let path = PathBuf::from(decoded);
    Ok(if path.is_absolute() {
        path
    } else {
        base_directory.join(path)
    })
}

fn image_uri_for_path(resolved: &Path) -> Result<String, String> {
    if !resolved.is_file() {
        return Err(format!("找不到图片：{}", resolved.display()));
    }
    let absolute = resolved
        .canonicalize()
        .unwrap_or_else(|_| resolved.to_owned());
    // egui_extras strips its file:// prefix but does not percent-decode the
    // remaining path. Preserve the native path, including Windows verbatim
    // prefixes and literal %, #, spaces and Unicode characters.
    let path = absolute
        .to_str()
        .ok_or_else(|| "图片路径不能表示为 Unicode".to_owned())?;
    Ok(if cfg!(windows) {
        format!("file:///{path}")
    } else {
        format!("file://{path}")
    })
}

pub(crate) fn decode_local_resource_path(destination: &str) -> Option<String> {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    if path_part.is_empty() || (has_uri_scheme(path_part) && !Path::new(path_part).is_absolute()) {
        return None;
    }
    percent_decode_path(path_part)
}

fn has_uri_scheme(value: &str) -> bool {
    let before_slash = value
        .find(['/', '\\'])
        .map_or(value, |index| &value[..index]);
    before_slash.contains(':')
}

fn percent_decode_path(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let high = bytes.get(cursor + 1).and_then(|value| hex_value(*value))?;
            let low = bytes.get(cursor + 2).and_then(|value| hex_value(*value))?;
            decoded.push((high << 4) | low);
            cursor += 3;
        } else {
            decoded.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn render_math_widget(
    ui: &mut Ui,
    cache: &mut HashMap<String, Arc<[u8]>>,
    math: &str,
    inline: bool,
    dark: bool,
) -> Response {
    let kind = if inline {
        "math-inline"
    } else {
        "math-display"
    };
    let key = generated_svg_key(kind, math, dark);
    let rendered = if let Some(bytes) = cache.get(&key) {
        Ok(bytes.clone())
    } else {
        markdown::render_math_svg(math, inline).and_then(|mut svg| {
            if dark {
                svg = svg
                    .replace("rgba(0,0,0,1)", "rgba(232,234,240,1)")
                    .replace("rgba(0, 0, 0, 1)", "rgba(232, 234, 240, 1)")
                    .replace("rgb(0,0,0)", "rgb(232,234,240)");
            }
            let bytes = Arc::<[u8]>::from(svg.into_bytes());
            cache_generated_svg(ui.ctx(), cache, key.clone(), bytes.clone())
                .then_some(bytes)
                .ok_or_else(|| "公式超过预览缓存资源预算".to_owned())
        })
    };
    show_generated_svg(ui, key, rendered, format!("公式错误：{math}"))
}

pub(crate) fn render_mermaid_widget(
    ui: &mut Ui,
    cache: &mut HashMap<String, Arc<[u8]>>,
    source: &str,
    dark: bool,
) -> Response {
    let key = generated_svg_key("mermaid", source, dark);
    let rendered = if let Some(bytes) = cache.get(&key) {
        Ok(bytes.clone())
    } else {
        markdown::render_mermaid_svg(source, dark).and_then(|svg| {
            let bytes = Arc::<[u8]>::from(svg.into_bytes());
            cache_generated_svg(ui.ctx(), cache, key.clone(), bytes.clone())
                .then_some(bytes)
                .ok_or_else(|| "图表超过预览缓存资源预算".to_owned())
        })
    };
    show_generated_svg(ui, key, rendered, "Mermaid 图表错误".to_owned())
}

fn show_generated_svg(
    ui: &mut Ui,
    key: String,
    rendered: Result<Arc<[u8]>, String>,
    error_prefix: String,
) -> Response {
    match rendered {
        Ok(bytes) => ui.add(
            egui::Image::new(egui::ImageSource::Bytes {
                uri: format!("bytes://rupora/{key}.svg").into(),
                bytes: egui::load::Bytes::Shared(bytes),
            })
            .fit_to_original_size(1.0)
            .max_width(ui.available_width()),
        ),
        Err(error) => ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{error_prefix}（{}）", error.replace('\n', " ")),
        ),
    }
}

fn generated_svg_key(kind: &str, source: &str, dark: bool) -> String {
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    format!("{kind}-{:016x}", hasher.finish())
}

pub(crate) fn cache_generated_svg(
    ctx: &Context,
    cache: &mut HashMap<String, Arc<[u8]>>,
    key: String,
    bytes: Arc<[u8]>,
) -> bool {
    if bytes.len() > MAX_GENERATED_SVG_CACHE_BYTES {
        return false;
    }
    let replacing = cache.contains_key(&key);
    while (!replacing && cache.len() >= MAX_GENERATED_SVG_CACHE_ENTRIES)
        || cache
            .values()
            .map(|value| value.len())
            .sum::<usize>()
            .saturating_sub(cache.get(&key).map_or(0, |existing| existing.len()))
            .saturating_add(bytes.len())
            > MAX_GENERATED_SVG_CACHE_BYTES
    {
        let Some(victim) = cache.keys().find(|candidate| *candidate != &key).cloned() else {
            return false;
        };
        cache.remove(&victim);
        ctx.forget_image(&format!("bytes://rupora/{victim}.svg"));
    }
    cache.insert(key, bytes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_image(path: &Path, width: u32, pixel: [u8; 4], timestamp: u64) {
        image::RgbaImage::from_pixel(width, 1, image::Rgba(pixel))
            .save(path)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp))
            .unwrap();
    }

    fn load_test_image(context: &Context, uri: &str) -> Arc<egui::ColorImage> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match context
                .try_load_image(uri, egui::load::SizeHint::default())
                .unwrap()
            {
                egui::load::ImagePoll::Ready { image } => return image,
                egui::load::ImagePoll::Pending { .. } => {
                    assert!(Instant::now() < deadline, "image loader timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn overwriting_a_local_image_refreshes_decoded_pixels_at_the_same_file_uri() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("图片 space #%🙂.png");
        let destination = "图片%20space%20%23%25🙂.png";
        write_test_image(&path, 1, [255, 0, 0, 255], 10);
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let mut store = LocalImageStore::new(Duration::ZERO);
        let block_id = markdown::blocks("image")[0].id;
        let before = store.resolve(&context, (1, block_id), directory.path(), destination);
        let before_uri = before.uri.unwrap();
        let before_image = load_test_image(&context, &before_uri);
        assert_eq!(before_image.size, [1, 1]);
        assert_eq!(before_image.pixels, [egui::Color32::RED]);

        write_test_image(&path, 2, [0, 0, 255, 255], 20);
        let after = store.resolve(&context, (1, block_id), directory.path(), destination);
        assert_eq!(after.uri.as_ref().unwrap(), &before_uri);
        assert_ne!(after.revision, before.revision);
        let after_image = load_test_image(&context, after.uri.as_ref().unwrap());
        assert_eq!(after_image.size, [2, 1]);
        assert_eq!(after_image.pixels, [egui::Color32::BLUE; 2]);
    }

    #[test]
    fn overwriting_a_local_image_with_unchanged_metadata_refreshes_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("same-stamp.png");
        write_test_image(&path, 1, [255, 0, 0, 255], 10);
        let original_metadata = std::fs::metadata(&path).unwrap();
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let mut store = LocalImageStore::new(Duration::ZERO);
        let block_id = markdown::blocks("image")[0].id;
        let before = store.resolve(&context, (1, block_id), directory.path(), "same-stamp.png");
        let uri = before.uri.as_ref().unwrap();
        assert_eq!(load_test_image(&context, uri).pixels, [egui::Color32::RED]);

        write_test_image(&path, 1, [0, 0, 255, 255], 10);
        let replacement_metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(original_metadata.len(), replacement_metadata.len());
        assert_eq!(
            original_metadata.modified().unwrap(),
            replacement_metadata.modified().unwrap()
        );
        let after = store.resolve(&context, (1, block_id), directory.path(), "same-stamp.png");
        assert_ne!(after.revision, before.revision);
        assert_eq!(load_test_image(&context, uri).pixels, [egui::Color32::BLUE]);
    }

    #[test]
    fn large_local_images_check_unchanged_metadata_off_the_ui_thread() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.png");
        let bytes = LOCAL_IMAGE_SYNCHRONOUS_HASH_BYTES as usize + 1;
        std::fs::write(&path, vec![b'a'; bytes]).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        file.set_modified(timestamp).unwrap();
        let context = Context::default();
        let mut store = LocalImageStore::new(Duration::ZERO);
        let owner = (1, markdown::blocks("image")[0].id);
        let first = store.resolve(&context, owner, directory.path(), "large.png");
        assert!(store.entries[&path].pending_content_hash.is_some());

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut baseline = first.clone();
        while store.entries[&path].content_hash.is_none() {
            assert!(Instant::now() < deadline, "initial image hash timed out");
            baseline = store.resolve(&context, owner, directory.path(), "large.png");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(baseline.revision, first.revision);

        std::fs::write(&path, vec![b'b'; bytes]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(timestamp)
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), bytes as u64);
        let pending = store.resolve(&context, owner, directory.path(), "large.png");
        assert_eq!(pending.revision, baseline.revision);
        assert!(store.entries[&path].pending_content_hash.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let refreshed = store.resolve(&context, owner, directory.path(), "large.png");
            if refreshed.revision != baseline.revision {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "replacement image hash timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn resource_checks_are_shared_throttled_and_released_with_the_last_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.png");
        write_test_image(&path, 1, [255, 0, 0, 255], 10);
        let context = Context::default();
        let mut store = LocalImageStore::new(Duration::from_secs(60));
        let block_id = markdown::blocks("image")[0].id;
        let first = store.resolve(&context, (1, block_id), directory.path(), "shared.png");
        write_test_image(&path, 2, [0, 0, 255, 255], 20);
        let second = store.resolve(&context, (2, block_id), directory.path(), "shared.png");
        assert_eq!(
            second.revision, first.revision,
            "same-path owners share the check interval"
        );
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.owners.len(), 2);

        store.forget_document(1);
        assert_eq!(store.entries.len(), 1);
        let retained = store.resolve(&context, (2, block_id), directory.path(), "shared.png");
        assert_eq!(retained.revision, first.revision);
        store.forget_document(2);
        assert!(store.entries.is_empty());
        assert!(store.owners.is_empty());

        let reopened = store.resolve(&context, (3, block_id), directory.path(), "shared.png");
        assert_ne!(reopened.revision, first.revision);
        store.resolve(
            &context,
            (3, block_id),
            directory.path(),
            "https://example.invalid/image.png",
        );
        assert!(
            store.entries.is_empty(),
            "a nonlocal replacement releases the old path"
        );
        assert!(store.owners.is_empty());
    }

    #[test]
    fn audit_local_image_uri_is_loadable_by_the_installed_egui_loader() {
        use egui::load::BytesPoll;
        use std::time::{Duration, Instant};

        let directory = tempfile::tempdir().unwrap();
        let filename = "图片 space #%🙂.png";
        let bytes = b"actual image bytes";
        std::fs::write(directory.path().join(filename), bytes).unwrap();
        let uri = document_image_uri(directory.path(), "图片%20space%20%23%25🙂.png").unwrap();
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match context.try_load_bytes(&uri).unwrap() {
                BytesPoll::Ready { bytes: loaded, .. } => {
                    assert_eq!(loaded.as_ref(), bytes);
                    break;
                }
                BytesPoll::Pending { .. } => {
                    assert!(Instant::now() < deadline, "image loader timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn recognizes_only_standalone_markdown_images() {
        let image = standalone_image("![审计截图](missing%20image.png)").unwrap();
        assert_eq!(image.alt, "审计截图");
        assert_eq!(image.destination, "missing%20image.png");
        assert!(standalone_image("before ![inline](image.png) after").is_none());
        assert!(standalone_image("![one](1.png) ![two](2.png)").is_none());
    }

    #[test]
    fn resolves_existing_local_images_and_reports_missing_or_remote_ones() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("present image.png"), b"image").unwrap();
        let uri = document_image_uri(directory.path(), "present%20image.png").unwrap();
        assert!(uri.starts_with("file://"));
        assert!(uri.ends_with("present image.png"));
        assert!(document_image_uri(directory.path(), "missing.png").is_err());
        assert!(document_image_uri(directory.path(), "https://example.invalid/image.png").is_err());
    }

    #[test]
    fn recognizes_standalone_display_math_and_mermaid() {
        let math = standalone_display_math("$$x^2 + y^2$$").unwrap();
        assert_eq!(math.source, "x^2 + y^2");
        assert!(standalone_display_math("before $x$ after").is_none());

        let diagram = standalone_mermaid("```mermaid\nflowchart LR\nA --> B\n```").unwrap();
        assert_eq!(diagram.source, "flowchart LR\nA --> B\n");
        assert!(standalone_mermaid("```rust\nlet x = 1;\n```").is_none());
    }

    #[test]
    fn rejects_invalid_percent_encoded_image_paths() {
        let directory = tempfile::tempdir().unwrap();
        assert!(document_image_uri(directory.path(), "bad%ZZ.png").is_err());
        assert_eq!(
            decode_local_resource_path("notes/My%20Note.md#part").as_deref(),
            Some("notes/My Note.md")
        );
        assert!(decode_local_resource_path("https://example.com/note.md").is_none());
    }
}
