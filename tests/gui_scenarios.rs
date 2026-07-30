use eframe::egui::{
    self, Event, Id, ImeEvent, RawInput, TextEdit,
    accesskit::{Action, Role},
};
use rupora::editor_buffer::{TrackingTextBuffer, set_accessible_label};

const EDITOR_LABEL: &str = "Markdown 源码编辑区";

fn run_editor_frame(
    context: &egui::Context,
    text: &mut String,
    events: Vec<Event>,
    request_focus: bool,
) -> egui::FullOutput {
    let input = RawInput {
        events,
        ..RawInput::default()
    };
    context.run_ui(input, |ui| {
        let id = Id::new("gui-scenario-editor");
        if request_focus {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let mut buffer = TrackingTextBuffer::new(text);
        TextEdit::multiline(&mut buffer)
            .id(id)
            .hint_text(EDITOR_LABEL)
            .show(ui);
        set_accessible_label(ui.ctx(), id, EDITOR_LABEL);
    })
}

#[test]
fn replays_chinese_ime_composition_and_followup_emoji_input() {
    let context = egui::Context::default();
    let mut text = String::new();

    run_editor_frame(&context, &mut text, Vec::new(), true);
    run_editor_frame(
        &context,
        &mut text,
        vec![Event::Ime(ImeEvent::Preedit {
            text: "你".to_owned(),
            active_range_chars: Some(0..1),
        })],
        false,
    );
    assert_eq!(text, "你");

    run_editor_frame(
        &context,
        &mut text,
        vec![Event::Ime(ImeEvent::Preedit {
            text: "你好".to_owned(),
            active_range_chars: Some(0..2),
        })],
        false,
    );
    assert_eq!(text, "你好");

    run_editor_frame(
        &context,
        &mut text,
        vec![Event::Ime(ImeEvent::Commit("你好".to_owned()))],
        false,
    );
    run_editor_frame(
        &context,
        &mut text,
        vec![Event::Text("🙂".to_owned())],
        false,
    );
    assert_eq!(text, "你好🙂");
}

#[test]
fn exposes_a_named_editable_accesskit_node() {
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut text = "# 可访问文档".to_owned();

    let output = run_editor_frame(&context, &mut text, Vec::new(), true);
    let update = output
        .platform_output
        .accesskit_update
        .expect("accessibility tree should be emitted");
    let editor = update
        .nodes
        .iter()
        .map(|(_, node)| node)
        .find(|node| node.role() == Role::MultilineTextInput)
        .expect("multiline editor node should exist");

    assert_eq!(editor.label(), Some(EDITOR_LABEL));
    assert!(editor.supports_action(Action::Focus));
    assert!(editor.text_selection().is_some());
}
