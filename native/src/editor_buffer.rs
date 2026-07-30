use eframe::egui::{self, TextBuffer, text::CharIndex};

/// A contiguous editor buffer that captures the pre-edit text only on the first mutation.
///
/// `egui::TextEdit` needs a contiguous `str` for layout. This adapter avoids cloning that
/// string on idle frames while still preserving the exact pre-edit value for document history.
pub struct TrackingTextBuffer<'a> {
    text: &'a mut String,
    before: Option<String>,
}

impl<'a> TrackingTextBuffer<'a> {
    pub fn new(text: &'a mut String) -> Self {
        Self { text, before: None }
    }

    pub fn take_before(&mut self) -> Option<String> {
        self.before.take()
    }

    fn remember_before(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.text.clone());
        }
    }
}

struct TrackingTextBufferType;

impl TextBuffer for TrackingTextBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.text
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        self.remember_before();
        TextBuffer::insert_text(self.text, text, char_index)
    }

    fn delete_char_range(&mut self, char_range: std::ops::Range<CharIndex>) {
        self.remember_before();
        TextBuffer::delete_char_range(self.text, char_range);
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<TrackingTextBufferType>()
    }
}

pub fn set_accessible_label(ctx: &egui::Context, id: egui::Id, label: impl Into<String>) {
    let label = label.into();
    ctx.accesskit_node_builder(id, |node| node.set_label(label));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_one_lazy_unicode_snapshot_per_edit() {
        let mut text = "你🙂好".to_owned();
        let mut buffer = TrackingTextBuffer::new(&mut text);
        assert!(buffer.take_before().is_none());

        TextBuffer::insert_text(&mut buffer, "!", CharIndex(2));
        TextBuffer::delete_char_range(&mut buffer, CharIndex(0)..CharIndex(1));

        assert_eq!(buffer.take_before().as_deref(), Some("你🙂好"));
        assert_eq!(TextBuffer::as_str(&buffer), "🙂!好");
    }
}
