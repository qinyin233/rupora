//! Prepare the semantic inputs of a block before deciding whether to paint it.
//! Callers never choose a cache fingerprint independently of the painted input.

use super::*;
use std::{cell::OnceCell, collections::HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RenderingDependencies {
    source: u64,
    preview: u64,
    references: u64,
    resource: u64,
}

impl RenderingDependencies {
    #[cfg(test)]
    pub(super) fn source_only(source: &str) -> Self {
        let hash = block_source_hash(source);
        Self {
            source: hash,
            preview: hash,
            references: 0,
            resource: 0,
        }
    }
}

pub(super) struct PreparedDocumentState {
    context: Rc<DocumentContext>,
    blocks: HashMap<BlockId, Rc<PreparedBlockData>>,
}

struct DocumentContext {
    source_hash: u64,
    references_revision: u64,
    references: Arc<markdown::ReferenceDefinitions>,
    front_matter: Option<markdown::FrontMatter>,
    toc: OnceCell<Arc<str>>,
}

struct PreparedBlockData {
    source_hash: u64,
    preview_hash: u64,
    document_source_hash: u64,
    references_revision: u64,
    at_document_start: bool,
    generated: Option<Arc<str>>,
    image: Option<crate::native_preview::NativeImage>,
    fenced_code: bool,
}

/// One immutable semantic context for a document layout pass. Preparing an
/// unchanged document reuses its context and the prepared data of every block.
pub(crate) struct PreparedDocument<'a> {
    cache: RenderCache,
    document_id: u64,
    source: &'a str,
    context: Rc<DocumentContext>,
}

/// The exact input used to estimate, paint and remember one block's height.
pub(crate) struct PreparedBlock<'a> {
    cache: RenderCache,
    key: HybridBlockLayoutKey,
    source: &'a str,
    data: Rc<PreparedBlockData>,
    references: Arc<markdown::ReferenceDefinitions>,
    dependencies: RenderingDependencies,
    image: Option<PreparedImage>,
    base_directory: &'a Path,
    width: f32,
    dark: bool,
}

impl RenderCache {
    pub(crate) fn prepare_document<'a>(
        &self,
        document_id: u64,
        source: &'a str,
        blocks: &[markdown::MarkdownBlock],
        references: Arc<markdown::ReferenceDefinitions>,
    ) -> PreparedDocument<'a> {
        let source_hash = block_source_hash(source);
        let mut state = self.state.borrow_mut();
        let previous = state.documents.get(&document_id);
        let same_references = previous.is_some_and(|previous| {
            Arc::ptr_eq(&previous.context.references, &references)
                || previous.context.references == references
        });
        let context = if let Some(previous) = previous
            && previous.context.source_hash == source_hash
            && same_references
        {
            previous.context.clone()
        } else {
            let references_revision = previous.map_or(1, |previous| {
                previous.context.references_revision + u64::from(!same_references)
            });
            let context = Rc::new(DocumentContext {
                source_hash,
                references_revision,
                references,
                front_matter: markdown::parse_front_matter(source),
                toc: OnceCell::new(),
            });
            let mut prepared_blocks = state
                .documents
                .remove(&document_id)
                .map_or_else(HashMap::new, |previous| previous.blocks);
            let live_blocks = blocks.iter().map(|block| block.id).collect::<HashSet<_>>();
            let removed_blocks = prepared_blocks
                .keys()
                .copied()
                .filter(|id| !live_blocks.contains(id))
                .collect::<Vec<_>>();
            prepared_blocks.retain(|id, _| live_blocks.contains(id));
            for block_id in removed_blocks {
                state
                    .resources
                    .local_images
                    .forget_block((document_id, block_id));
            }
            state.documents.insert(
                document_id,
                PreparedDocumentState {
                    context: context.clone(),
                    blocks: prepared_blocks,
                },
            );
            context
        };
        PreparedDocument {
            cache: self.clone(),
            document_id,
            source,
            context,
        }
    }
}

impl<'a> PreparedDocument<'a> {
    pub(crate) fn block(
        &self,
        context: &egui::Context,
        block: &markdown::MarkdownBlock,
        width: f32,
        dark: bool,
        base_directory: &'a Path,
    ) -> PreparedBlock<'a> {
        let source = &self.source[block.range.clone()];
        let source_hash = block_source_hash(source);
        let at_document_start = block.range.start == 0;
        let mut state = self.cache.state.borrow_mut();
        let document = state
            .documents
            .get_mut(&self.document_id)
            .expect("a prepared document has a render context");
        let cached = document.blocks.get(&block.id).filter(|cached| {
            cached.source_hash == source_hash
                && cached.references_revision == self.context.references_revision
                && cached.at_document_start == at_document_start
                && (cached.generated.is_none()
                    || cached.document_source_hash == self.context.source_hash)
        });
        let data = if let Some(cached) = cached {
            cached.clone()
        } else {
            let generated = if at_document_start
                && let Some(front) = &self.context.front_matter
                && block.range.end <= front.body_start
            {
                Some(Arc::<str>::from(markdown::front_matter_preview_markdown(
                    front,
                )))
            } else if source.trim().eq_ignore_ascii_case("[TOC]")
                && markdown::is_toc_marker(self.source, block.range.clone())
            {
                Some(
                    self.context
                        .toc
                        .get_or_init(|| Arc::from(markdown::toc_preview_markdown(self.source)))
                        .clone(),
                )
            } else {
                None
            };
            let preview_source = generated.as_deref().unwrap_or(source);
            let data = Rc::new(PreparedBlockData {
                source_hash,
                preview_hash: block_source_hash(preview_source),
                document_source_hash: self.context.source_hash,
                references_revision: self.context.references_revision,
                at_document_start,
                image: standalone_image_with_references(preview_source, &self.context.references),
                fenced_code: crate::wysiwyg::fenced_code_content(preview_source).is_some(),
                generated,
            });
            document.blocks.insert(block.id, data.clone());
            data
        };
        let image = if let Some(image) = data.image.as_ref() {
            Some(PreparedImage {
                image: image.clone(),
                resource: state.resources.local_images.resolve(
                    context,
                    (self.document_id, block.id),
                    base_directory,
                    &image.destination,
                ),
            })
        } else {
            state
                .resources
                .local_images
                .forget_block((self.document_id, block.id));
            None
        };
        let dependencies = RenderingDependencies {
            source: data.source_hash,
            preview: data.preview_hash,
            references: data.references_revision,
            resource: image.as_ref().map_or(0, |image| image.resource.revision),
        };
        let mut key = HybridBlockLayoutKey::new(self.document_id, block.id, width, dark);
        key.pixels_per_point = context.pixels_per_point().to_bits();
        PreparedBlock {
            cache: self.cache.clone(),
            key,
            source,
            data,
            references: self.context.references.clone(),
            dependencies,
            image,
            base_directory,
            width,
            dark,
        }
    }
}

impl PreparedBlock<'_> {
    pub(crate) fn preview_source(&self) -> &str {
        self.data.generated.as_deref().unwrap_or(self.source)
    }

    pub(crate) fn is_generated(&self) -> bool {
        self.data.generated.is_some()
    }

    pub(crate) fn estimated_height(&self) -> f32 {
        self.cache
            .state
            .borrow()
            .block_heights
            .get(&self.key)
            .filter(|cached| cached.dependencies == self.dependencies)
            .map(|cached| cached.height)
            .unwrap_or_else(|| {
                estimated_hybrid_block_height(
                    self.preview_source(),
                    self.width,
                    self.data.fenced_code,
                )
            })
    }

    pub(crate) fn remember_height(&self, height: f32) {
        self.cache.state.borrow_mut().block_heights.insert(
            self.key,
            CachedBlockHeight {
                dependencies: self.dependencies,
                height,
            },
        );
    }

    pub(crate) fn show(&self, ui: &mut Ui) -> NativeBlockPreview {
        show_native_block_preview_with_context(
            ui,
            self.preview_source(),
            &mut self.cache.state.borrow_mut().resources,
            BlockPreviewContext {
                owner: (self.key.document_id, self.key.block_id),
                base_directory: self.base_directory,
                dark: self.dark,
                palette: app_palette(self.dark),
                references: self.references.clone(),
                image: self.image.as_ref(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_prepared_blocks_are_reused_and_removed_blocks_do_not_accumulate() {
        let original = (0..256)
            .map(|index| format!("Paragraph {index}\n\n"))
            .collect::<String>();
        let mut index = markdown::BlockIndex::new(&original);
        let context = egui::Context::default();
        let cache = RenderCache::default();
        let prepared = cache.prepare_document(1, &original, index.blocks(), index.references());
        let first = prepared.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        for block in index.blocks() {
            prepared.block(&context, block, 400.0, false, Path::new("."));
        }
        for _ in 0..20 {
            let repeated = cache.prepare_document(1, &original, index.blocks(), index.references());
            assert!(Rc::ptr_eq(&prepared.context, &repeated.context));
            let block = repeated.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
            assert!(Rc::ptr_eq(&first.data, &block.data));
        }
        assert_eq!(
            cache.state.borrow().documents[&1].blocks.len(),
            index.blocks().len()
        );

        let shortened = "Paragraph 0\n\nChanged paragraph";
        index.update(shortened);
        let updated = cache.prepare_document(1, shortened, index.blocks(), index.references());
        let unchanged = updated.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        assert!(
            Rc::ptr_eq(&first.data, &unchanged.data),
            "an unrelated edit does not reparse a plain block"
        );
        for block in index.blocks() {
            updated.block(&context, block, 400.0, false, Path::new("."));
        }
        assert_eq!(cache.state.borrow().documents[&1].blocks.len(), 2);
        cache.forget_document(1);
        assert!(cache.state.borrow().documents.is_empty());
    }

    #[test]
    fn reference_context_changes_invalidate_an_unchanged_block_measurement() {
        let original = "[label][ref]\n\n[ref]: first.md";
        let updated = "[label][ref]\n\n[other]: first.md";
        let mut index = markdown::BlockIndex::new(original);
        let block_id = index.blocks()[0].id;
        let context = egui::Context::default();
        let cache = RenderCache::default();
        let document = cache.prepare_document(1, original, index.blocks(), index.references());
        let before = document.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        before.remember_height(73.0);
        assert_eq!(before.estimated_height(), 73.0);

        index.update(updated);
        assert_eq!(index.blocks()[0].id, block_id);
        let document = cache.prepare_document(1, updated, index.blocks(), index.references());
        let after = document.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        assert_eq!(after.preview_source(), before.preview_source());
        assert_ne!(after.estimated_height(), 73.0);
        after.remember_height(91.0);
        let document = cache.prepare_document(
            1,
            updated,
            index.blocks(),
            markdown::reference_definitions(updated),
        );
        let unchanged = document.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        assert_eq!(
            unchanged.estimated_height(),
            91.0,
            "equivalent reference maps preserve the measurement"
        );
    }

    #[test]
    fn unrelated_prose_edits_preserve_the_effective_toc_measurement() {
        let original = "[TOC]\n\n# Heading\n\nFirst paragraph";
        let updated = "[TOC]\n\n# Heading\n\nRewritten paragraph";
        let mut index = markdown::BlockIndex::new(original);
        let context = egui::Context::default();
        let cache = RenderCache::default();
        let document = cache.prepare_document(1, original, index.blocks(), index.references());
        let before = document.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        before.remember_height(73.0);
        index.update(updated);
        let document = cache.prepare_document(1, updated, index.blocks(), index.references());
        let after = document.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        assert_eq!(after.preview_source(), before.preview_source());
        assert_eq!(after.estimated_height(), 73.0);
    }

    #[test]
    fn a_changed_local_image_invalidates_its_height_without_changing_the_source_or_uri() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();
        let source = "![diagram](image.png)";
        let index = markdown::BlockIndex::new(source);
        let context = egui::Context::default();
        let cache = RenderCache::default();
        cache.state.borrow_mut().resources.local_images = LocalImageStore::new(Duration::ZERO);
        let document = cache.prepare_document(1, source, index.blocks(), index.references());
        let before = document.block(&context, &index.blocks()[0], 400.0, false, directory.path());
        before.remember_height(73.0);
        let before_uri = before.image.as_ref().unwrap().resource.uri.clone();

        std::fs::write(&path, b"a replacement with different dimensions").unwrap();
        let after = document.block(&context, &index.blocks()[0], 400.0, false, directory.path());
        assert_eq!(after.preview_source(), before.preview_source());
        assert_eq!(after.image.as_ref().unwrap().resource.uri, before_uri);
        assert_ne!(after.estimated_height(), 73.0);
        after.remember_height(147.0);
        assert_eq!(
            document
                .block(&context, &index.blocks()[0], 400.0, false, directory.path())
                .estimated_height(),
            147.0
        );
    }

    #[test]
    fn deleting_an_image_block_releases_its_resource_even_without_painting() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("image.png"), b"image").unwrap();
        let source = "# Heading\n\n![diagram](image.png)";
        let mut index = markdown::BlockIndex::new(source);
        let context = egui::Context::default();
        let cache = RenderCache::default();
        let document = cache.prepare_document(1, source, index.blocks(), index.references());
        let before = document.block(&context, &index.blocks()[1], 400.0, false, directory.path());
        let original_revision = before.image.as_ref().unwrap().resource.revision;

        index.update("# Heading");
        cache.prepare_document(1, "# Heading", index.blocks(), index.references());
        index.update(source);
        let document = cache.prepare_document(1, source, index.blocks(), index.references());
        let restored = document.block(&context, &index.blocks()[1], 400.0, false, directory.path());
        assert_ne!(
            restored.image.as_ref().unwrap().resource.revision,
            original_revision
        );
        let restored_revision = restored.image.as_ref().unwrap().resource.revision;

        cache.forget_document(1);
        let document = cache.prepare_document(1, source, index.blocks(), index.references());
        let reopened = document.block(&context, &index.blocks()[1], 400.0, false, directory.path());
        assert_ne!(
            reopened.image.as_ref().unwrap().resource.revision,
            restored_revision
        );
    }

    #[test]
    fn layout_measurements_distinguish_fractional_widths() {
        let source = "A wrapping paragraph";
        let index = markdown::BlockIndex::new(source);
        let context = egui::Context::default();
        let cache = RenderCache::default();
        let document = cache.prepare_document(1, source, index.blocks(), index.references());
        let narrow = document.block(&context, &index.blocks()[0], 400.1, false, Path::new("."));
        narrow.remember_height(73.0);
        let wider = document.block(&context, &index.blocks()[0], 400.4, false, Path::new("."));
        assert_ne!(wider.estimated_height(), 73.0);
    }
}
