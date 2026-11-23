//! Ratatui's Crossterm VT encoder with an in-process sink and explicit host metrics.
use super::vt::{self, Vt};
use crate::app::{AppState, View};
use ratatui::{
    Terminal,
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Position, Rect, Size},
};
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
    time::Instant,
};

/// Host dimensions in cells and physical pixels, independent of stdout and stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub columns: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}
/// Host colors for default-color terminal cells, including native System appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub system: Option<crate::app::theme_palette::Palette<[u8; 3]>>,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
}
/// One monotonically ordered native viewport measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub metrics: Metrics,
    pub scale_factor: f32,
    pub revision: u64,
    pub appearance: Appearance,
    pub geometry: crate::ui::layout::WindowGeometry,
}
struct Sink(Rc<RefCell<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Output {
    encoder: CrosstermBackend<Sink>,
    pending: Rc<RefCell<Vec<u8>>>,
    metrics: Metrics,
    cursor: Position,
}
impl Backend for Output {
    fn draw<'a, I>(&mut self, cells: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.encoder.draw(cells)
    }
    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.encoder.append_lines(n)
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.encoder.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.encoder.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.cursor = position.into();
        self.encoder.set_cursor_position(self.cursor)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.encoder.clear()
    }
    fn clear_region(&mut self, kind: ClearType) -> io::Result<()> {
        self.encoder.clear_region(kind)
    }
    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.metrics.columns, self.metrics.rows))
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size()?,
            pixels: Size::new(
                self.metrics.columns.saturating_mul(self.metrics.cell_width),
                self.metrics.rows.saturating_mul(self.metrics.cell_height),
            ),
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.encoder)
    }
}

/// A disposable presentation adapter; application state and playback are borrowed.
pub struct Renderer {
    terminal: Terminal<Output>,
    vt: Vt,
    motion: crate::ui::motion::SceneMotion,
    metrics: Metrics,
    motion_initialized: bool,
    artwork_mode: Option<crate::app::state::ArtworkMode>,
    appearance: Option<Appearance>,
    animating: bool,
    geometry: crate::ui::layout::WindowGeometry,
}
impl Renderer {
    /// Create a VT surface without a PTY, input reader, raw mode or application session.
    /// # Errors
    /// Propagates Ratatui backend and terminal-library initialization failures.
    pub fn new(metrics: Metrics) -> anyhow::Result<Self> {
        let metrics = valid_metrics(metrics);
        let mut vt = Vt::new(metrics.columns, metrics.rows)?;
        vt.resize(
            metrics.columns,
            metrics.rows,
            u32::from(metrics.cell_width),
            u32::from(metrics.cell_height),
        )?;
        crossterm::style::force_color_output(true);
        let pending = Rc::new(RefCell::new(Vec::new()));
        let terminal = Terminal::new(Output {
            encoder: CrosstermBackend::new(Sink(pending.clone())),
            pending,
            metrics,
            cursor: Position::new(0, 0),
        })?;
        initialize_artwork(metrics);
        Ok(Self {
            terminal,
            vt,
            metrics,
            motion: crate::ui::motion::SceneMotion::new(true),
            motion_initialized: false,
            artwork_mode: None,
            appearance: None,
            animating: false,
            geometry: Default::default(),
        })
    }
    /// Apply window geometry without storing it in application domain state.
    pub fn set_geometry(&mut self, geometry: crate::ui::layout::WindowGeometry) {
        self.geometry = geometry;
    }
    /// Prepare a presentation handoff at its final panel and navigation geometry.
    pub fn settle_motion(&mut self) {
        self.motion = crate::ui::motion::SceneMotion::new(true);
        self.motion_initialized = false;
        self.animating = false;
    }
    /// Whether the last frame requires another animation sample before settling.
    pub fn animating(&self) -> bool {
        self.animating
    }
    /// Resolve default foreground/background colors using the window's current appearance.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        if self.appearance == Some(appearance) {
            return;
        }
        self.appearance = Some(appearance);
        crate::ui::theme::set_host_palette(appearance.system);
        let [fr, fg, fb] = appearance.foreground;
        let [br, bg, bb] = appearance.background;
        self.vt.write(format!("\x1b]10;rgb:{fr:02x}/{fg:02x}/{fb:02x}\x1b\\\x1b]11;rgb:{br:02x}/{bg:02x}/{bb:02x}\x1b\\").as_bytes());
    }
    /// Resize both the VT emulator and Ratatui using the same cell and pixel metrics.
    /// # Errors
    /// Propagates encoder and terminal allocation errors.
    pub fn resize(&mut self, metrics: Metrics) -> anyhow::Result<()> {
        let metrics = valid_metrics(metrics);
        if metrics == self.metrics {
            return Ok(());
        }
        self.metrics = metrics;
        self.terminal.backend_mut().metrics = metrics;
        self.terminal
            .resize(Rect::new(0, 0, metrics.columns, metrics.rows))?;
        self.vt.resize(
            metrics.columns,
            metrics.rows,
            u32::from(metrics.cell_width),
            u32::from(metrics.cell_height),
        )?;
        self.vt.write(b"\x1b_Ga=d,d=A,q=2;\x1b\\");
        initialize_artwork(metrics);
        Ok(())
    }
    /// Draw the existing TUI and publish the resulting Ghostty render-state snapshot.
    /// # Errors
    /// Propagates output, VT update and grapheme decoding failures.
    pub fn draw(&mut self, state: &mut AppState) -> anyhow::Result<vt::Frame> {
        state.terminal_width = self.metrics.columns;
        state.terminal_height = self.metrics.rows;
        let now = Instant::now();
        let player = state.view == View::NowPlaying
            || (state.view == View::Queue
                && state.tui_queue_workspace == crate::app::state::TuiWorkspace::NowPlaying);
        if !self.motion_initialized {
            state.tui_motion.initialize(
                state.tui_sidebar_visible,
                state.tui_queue_visible,
                player,
                now,
            );
            self.motion_initialized = true;
        }
        state.tui_motion.sync(
            state.tui_sidebar_visible,
            state.tui_queue_visible,
            player,
            now,
        );
        let host = crate::ui::OutputHost::Embedded {
            cell_width: self.metrics.cell_width,
            cell_height: self.metrics.cell_height,
            geometry: self.geometry,
        };
        let artwork_mode = crate::ui::effective_artwork_mode(state, self.motion.active(now), host);
        if self.artwork_mode != Some(artwork_mode) {
            self.vt.write(b"\x1b_Ga=d,d=A,q=2;\x1b\\");
            initialize_artwork(self.metrics);
            self.artwork_mode = Some(artwork_mode);
        }
        let mut feedback = None;
        self.terminal.draw(|frame| {
            feedback = Some(crate::ui::render_with_motion(
                frame,
                state,
                &mut self.motion,
                host,
            ));
        })?;
        if let Some(feedback) = feedback {
            state.apply_render_feedback(feedback);
        }
        self.animating = self.motion.active(now) || state.tui_motion.active(now);
        let bytes = std::mem::take(&mut *self.terminal.backend_mut().pending.borrow_mut());
        self.vt.write(&bytes);
        Ok(self.vt.frame()?)
    }
}
fn valid_metrics(metrics: Metrics) -> Metrics {
    Metrics {
        columns: metrics.columns.max(1),
        rows: metrics.rows.max(1),
        cell_width: metrics.cell_width.max(1),
        cell_height: metrics.cell_height.max(1),
    }
}

// The pinned image picker has no nondeprecated constructor for explicit host
// metrics; its query constructor reads process stdio, which this adapter cannot use.
#[allow(deprecated)]
fn initialize_artwork(metrics: Metrics) {
    crate::ui::reset_artwork_presentation();
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize((metrics.cell_width, metrics.cell_height));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    crate::ui::artwork::init_grid_renderer(picker.clone());
    crate::ui::screens::now_playing::init_artwork_renderer(picker.clone());
    crate::ui::init_bio_artwork_renderer(picker);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholder_artwork_is_uploaded_again_after_a_renderer_handoff() {
        let metrics = Metrics {
            columns: 130,
            rows: 40,
            cell_width: 18,
            cell_height: 40,
        };
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.artwork.mode = crate::app::state::ArtworkMode::Auto;
        for _ in 0..3 {
            let mut renderer = Renderer::new(metrics).unwrap();
            let frame = renderer.draw(&mut state).unwrap();
            assert!(
                frame.images.len() >= 2,
                "large placeholder and transport thumbnail must both be placed"
            );
        }
    }

    #[test]
    fn view_options_preserves_images_and_occludes_their_covered_cells() {
        use crate::app::state::{ColumnSortMode, SortColumnType, SortPopupState};
        let metrics = Metrics {
            columns: 130,
            rows: 40,
            cell_width: 18,
            cell_height: 40,
        };
        let mut renderer = Renderer::new(metrics).unwrap();
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        let before = renderer.draw(&mut state).unwrap();
        assert!(!before.images.is_empty());
        state.popups.set_sort(Some(SortPopupState::new(
            0,
            "Albums".into(),
            SortColumnType::Album,
            ColumnSortMode::Default,
            true,
            false,
        )));
        let open = renderer.draw(&mut state).unwrap();
        assert!(
            !open.images.is_empty(),
            "opening View Options must retain image encoding"
        );
        let popup = state.hit_regions.popup_outer.unwrap();
        for image in &open.images {
            let area = Rect::new(
                image.column.max(0) as u16,
                image.row.max(0) as u16,
                image.size[0].div_ceil(u32::from(metrics.cell_width)) as u16,
                image.size[1].div_ceil(u32::from(metrics.cell_height)) as u16,
            );
            assert!(
                area.intersection(popup).is_empty(),
                "image placement must be clipped behind the popover: {area:?}, {popup:?}"
            );
        }
        state.popups.set_sort(None);
        let after = renderer.draw(&mut state).unwrap();
        assert!(!after.images.is_empty());
    }

    #[test]
    fn graphics_mode_publishes_a_native_visualizer_and_character_modes_retain_terminal_canvas() {
        let mut renderer = Renderer::new(Metrics {
            columns: 130,
            rows: 40,
            cell_width: 18,
            cell_height: 40,
        })
        .unwrap();
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        for mode in [
            crate::app::state::ArtworkMode::Auto,
            crate::app::state::ArtworkMode::Halfblocks,
            crate::app::state::ArtworkMode::Braille,
        ] {
            state.artwork.mode = mode;
            renderer.draw(&mut state).unwrap();
            let canvas = state
                .hit_regions
                .now_playing_content
                .as_ref()
                .unwrap()
                .visualizer_content_area;
            assert_eq!(
                state.hit_regions.graphical_visualizer,
                (mode == crate::app::state::ArtworkMode::Auto).then_some(canvas)
            );
            assert_eq!(state.hit_regions.transport.as_ref().unwrap().area.height, 5);
        }
    }

    #[test]
    fn handoff_settles_panels_without_disabling_subsequent_navigation_motion() {
        let mut renderer = Renderer::new(Metrics {
            columns: 130,
            rows: 38,
            cell_width: 18,
            cell_height: 40,
        })
        .unwrap();
        let mut state = AppState::new();
        state.tui_queue_visible = false;
        renderer.draw(&mut state).unwrap();
        state.view = View::NowPlaying;
        state.tui_queue_visible = true;
        renderer.settle_motion();
        renderer.draw(&mut state).unwrap();
        assert!(!renderer.animating());
        let now = Instant::now();
        assert_eq!(state.tui_motion.player(true, now), 1.);
        assert_eq!(state.tui_motion.queue(true, now), 1.);
        state.tui_queue_visible = false;
        renderer.draw(&mut state).unwrap();
        assert!(renderer.animating());
    }

    #[test]
    fn repeated_artwork_has_independent_thumbnail_and_album_placements() {
        let mut renderer = Renderer::new(Metrics {
            columns: 80,
            rows: 30,
            cell_width: 18,
            cell_height: 40,
        })
        .unwrap();
        let pixels = image::RgbaImage::from_pixel(32, 32, image::Rgba([12, 34, 56, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        renderer
            .terminal
            .draw(|frame| {
                assert!(crate::ui::artwork::render_grid_image(
                    frame,
                    Rect::new(1, 1, 20, 10),
                    "album",
                    png.get_ref()
                ));
                assert!(crate::ui::artwork::render_grid_image(
                    frame,
                    Rect::new(30, 20, 4, 2),
                    "album",
                    png.get_ref()
                ));
            })
            .unwrap();
        let bytes = std::mem::take(&mut *renderer.terminal.backend_mut().pending.borrow_mut());
        renderer.vt.write(&bytes);
        let frame = renderer.vt.frame().unwrap();
        let identities = frame
            .images
            .iter()
            .map(|image| image.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(identities.len(), 2);
        let widths = frame
            .images
            .iter()
            .map(|image| image.size[0])
            .collect::<std::collections::BTreeSet<_>>();
        assert!(widths.len() > 1);
    }
    #[test]
    fn editing_a_settings_form_retains_unchanged_terminal_rows() {
        let mut renderer = Renderer::new(Metrics {
            columns: 130,
            rows: 38,
            cell_width: 18,
            cell_height: 40,
        })
        .unwrap();
        let mut state = AppState::new();
        state.view = View::Settings;
        state
            .popups
            .set_library_dialog(Some(crate::app::sources::dialogs::Dialog::Webdav(
                crate::app::sources::dialogs::WebdavForm::new(None),
            )));
        for _ in 0..20 {
            let frame = renderer.draw(&mut state).unwrap();
            let text = frame
                .cells
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Settings"), "{text}");
            assert!(text.contains("Username"), "{text}");
            crate::app::terminal_text::insert("a", &mut state, &crate::config::Config::default());
        }
    }
    #[test]
    fn column_insertion_keeps_unmodified_rows_in_every_published_frame() {
        let mut renderer = Renderer::new(Metrics {
            columns: 130,
            rows: 38,
            cell_width: 18,
            cell_height: 40,
        })
        .unwrap();
        let mut state = AppState::new();
        state.artist_nav.reset(
            "Artists",
            ('A'..='Z')
                .map(|letter| crate::app::state::BrowseItem::Artist {
                    key: letter.to_string(),
                    title: format!("{letter} Artist"),
                    thumb: None,
                    is_placeholder: false,
                })
                .collect(),
        );
        renderer.draw(&mut state).unwrap();
        state.artist_nav.drill_column(
            crate::app::state::BrowseColumn::new("Albums", vec![]),
            false,
        );
        for _ in 0..20 {
            let frame = renderer.draw(&mut state).unwrap();
            let text = frame
                .cells
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("C Artist"), "{text}");
            assert!(text.contains("Z Artist"), "{text}");
        }
    }
    #[test]
    fn actual_ratatui_output_draws_the_library_without_stdio_queries() {
        let mut renderer = Renderer::new(Metrics {
            columns: 100,
            rows: 35,
            cell_width: 9,
            cell_height: 18,
        })
        .unwrap();
        let mut state = AppState::new();
        state.artist_nav.reset(
            "Artists",
            vec![crate::app::state::BrowseItem::Artist {
                key: "artist".into(),
                title: "Unicode é 界".into(),
                thumb: None,
                is_placeholder: false,
            }],
        );
        let frame = renderer.draw(&mut state).unwrap();
        let text = frame
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert!(text.contains("Unicode é 界"));
        assert!(state.hit_regions.transport.is_some());
        for _ in 0..5 {
            let next = renderer.draw(&mut state).unwrap();
            let next_text = next
                .cells
                .iter()
                .flat_map(|row| row.iter())
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            assert_eq!(next_text, text);
        }

        state.artist_nav.drill_column(
            crate::app::state::BrowseColumn::new(
                "Albums",
                vec![crate::app::state::BrowseItem::Artist {
                    key: "second".into(),
                    title: "Second column".into(),
                    thumb: None,
                    is_placeholder: false,
                }],
            ),
            false,
        );
        for _ in 0..8 {
            let frame = renderer.draw(&mut state).unwrap();
            assert!(
                frame
                    .cells
                    .iter()
                    .flat_map(|row| row.iter())
                    .any(|cell| cell.text == "A")
            );
        }
        renderer
            .resize(Metrics {
                columns: 70,
                rows: 22,
                cell_width: 18,
                cell_height: 36,
            })
            .unwrap();
        assert_eq!(renderer.draw(&mut state).unwrap().columns, 70);
    }
}
