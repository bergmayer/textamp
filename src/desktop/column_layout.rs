//! Equal-width allocation for stock Kit resizers inside a horizontal column strip.
use super::*;
use gpui_kit::component::resizable::ResizableState;

pub(super) const MIN_COLUMN: f32 = 260.;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Layout {
    group: EntityId,
    columns: Vec<String>,
    inspector: bool,
    available: u32,
}

impl Desktop {
    pub(super) fn size_columns(
        &mut self,
        panels: &Entity<ResizableState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = Layout {
            group: panels.entity_id(),
            available: f32::from(panels.read(cx).container_size()).round().max(0.) as u32,
            columns: self
                .snapshot
                .columns
                .iter()
                .filter(|column| !column.hidden && !column.metadata)
                .map(|column| column.identity.clone())
                .collect(),
            inspector: self.snapshot.has_inspector(),
        };
        if self.column_layout.as_ref() == Some(&layout) {
            return;
        }
        self.column_layout = Some(layout.clone());
        let panels = panels.clone();
        let owner = cx.entity().downgrade();
        window.defer(cx, move |_, cx| {
            let _ = owner.update(cx, |this, cx| {
                if this.column_layout.as_ref() != Some(&layout) {
                    return;
                }
                let available = panels.read(cx).container_size();
                let count = layout.columns.len() + usize::from(layout.inspector);
                if available <= px(0.) || count == 0 {
                    this.column_layout = None;
                    return;
                }
                if let Some(layout) = this.column_layout.as_mut() {
                    layout.available = f32::from(available).round().max(0.) as u32;
                }
                this.column_reveal = Some(count - 1);
                panels.update(cx, |panels, cx| {
                    panels.clear();
                    for index in 0..count {
                        panels.insert_panel(Some(available / (index + 1) as f32), None, cx);
                    }
                });
                cx.notify();
            });
        });
    }
}
