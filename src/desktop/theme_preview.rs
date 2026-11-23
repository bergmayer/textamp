//! Theme selection previews use Kit navigation without applying a preference until confirmation.
use crate::app::theme::ThemeName;
use gpui_kit::component::{
    IndexPath,
    list::{ListDelegate, ListEvent, ListItem, ListState},
};
use gpui_kit::{App, Context, InteractiveElement, IntoElement, ParentElement, Styled, Window};

pub(super) struct Choices;
impl ListDelegate for Choices {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        ThemeName::all().len()
    }
    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<ListItem> {
        let name = *ThemeName::all().get(ix.row)?;
        let list = cx.entity().downgrade();
        Some(
            ListItem::new(("theme-choice", ix.row))
                .debug_selector(move || format!("theme-choice-{}", ix.row))
                .accessibility_label(format!("Preview {}", name.display_name()))
                .child(name.display_name())
                .on_click(move |_, window, cx| {
                    let _ = list.update(cx, |list, cx| {
                        let highlighted = list.selected_index() == Some(ix);
                        list.focus(window, cx);
                        list.set_selected_index(Some(ix), window, cx);
                        cx.emit(if highlighted {
                            ListEvent::Confirm(ix)
                        } else {
                            ListEvent::Select(ix)
                        });
                        cx.notify();
                    });
                    cx.stop_propagation();
                }),
        )
    }
    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
    }
}

pub(super) fn sample(name: ThemeName) -> impl IntoElement {
    let colors = super::theme::colors(name);
    let selection_text = super::theme::selection_foreground(colors.foreground, colors.list_active);
    gpui_kit::div()
        .debug_selector(|| "theme-preview".into())
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(colors.background)
        .text_color(colors.foreground)
        .border_1()
        .border_color(colors.border)
        .rounded_md()
        .child(
            gpui_kit::div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(name.display_name()),
        )
        .child(gpui_kit::div().text_color(colors.primary).child("Library"))
        .child(
            gpui_kit::div()
                .p_2()
                .rounded_sm()
                .bg(colors.list_active)
                .text_color(selection_text)
                .child(crate::app::preferences::THEME_PREVIEW_ALBUM),
        )
        .children(
            crate::app::preferences::THEME_PREVIEW_TRACKS
                .into_iter()
                .map(|(title, duration)| {
                    gpui_kit::div()
                        .flex()
                        .justify_between()
                        .gap_2()
                        .child(title)
                        .child(duration)
                }),
        )
        .child(
            gpui_kit::div()
                .mt_2()
                .h_1()
                .rounded_sm()
                .bg(colors.muted)
                .child(gpui_kit::div().h_full().w_2_3().bg(colors.primary)),
        )
        .child(
            gpui_kit::div()
                .text_color(colors.muted_foreground)
                .child("Playing · 2:10"),
        )
}
