//! Theme-derived application icons, with native Dock and Finder integration.
use crate::app::theme::ThemeName;
use crate::icon::{Appearance, Platform};
use image::ImageFormat;
use std::io::Cursor;

fn render(theme: ThemeName, appearance: Appearance) -> anyhow::Result<Vec<u8>> {
    let image = crate::icon::render(theme, appearance, 512, Platform::MacOs);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}

#[cfg(not(test))]
#[derive(Clone, Copy)]
struct CurrentIconTheme(ThemeName, Appearance);
#[cfg(not(test))]
impl gpui_kit::Global for CurrentIconTheme {}

#[cfg(not(test))]
pub(super) fn apply(theme: ThemeName, cx: &mut gpui_kit::App) {
    let appearance =
        if theme == ThemeName::System && gpui_kit::component::Theme::global(cx).is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        };
    if cx
        .try_global::<CurrentIconTheme>()
        .is_some_and(|current| current.0 == theme && current.1 == appearance)
    {
        return;
    }
    cx.set_global(CurrentIconTheme(theme, appearance));
    let task = cx
        .background_executor()
        .spawn(async move { render(theme, appearance) });
    cx.spawn(async move |cx| {
        let bytes = match task.await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error,"Unable to render application icon");
                return;
            }
        };
        cx.update(|cx| {
            if cx.global::<CurrentIconTheme>().0 != theme
                || cx.global::<CurrentIconTheme>().1 != appearance
            {
                return;
            }
            unsafe extern "C" {
                fn textamp_set_theme_icon(bytes: *const u8, length: usize) -> u32;
            }
            // The byte slice remains valid for the native call, which copies its data.
            let applied = unsafe { textamp_set_theme_icon(bytes.as_ptr(), bytes.len()) };
            if applied & 1 == 0 {
                tracing::warn!("Unable to update Dock icon");
            }
            if applied & 2 == 0 {
                tracing::debug!("Finder icon requires a writable application bundle");
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dock_icon_is_a_transparent_png() {
        let image =
            image::load_from_memory(&render(ThemeName::Platinum, Appearance::Light).unwrap())
                .unwrap()
                .into_rgba8();
        assert_eq!(image.dimensions(), (512, 512));
        assert_eq!(image[(0, 0)][3], 0);
    }
}
