//! Ordered preparation and activation of windowed frontend adapters.
use super::launch::WindowedMode;

/// Identity of the presentation allowed to commit input and layout feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity(pub u64);

/// Library content retained beneath windowed settings and other transient surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    #[default]
    Library,
    NowPlaying,
}

/// One destination preparation, retaining the active presentation until ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preparation {
    pub identity: Identity,
    pub destination: WindowedMode,
}

/// Preparation can wait for a platform-owned modal without discarding the request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transition {
    #[default]
    Idle,
    Preparing(Preparation),
    Deferred(Preparation),
}
impl Transition {
    fn request(self) -> Option<Preparation> {
        match self {
            Self::Idle => None,
            Self::Preparing(request) | Self::Deferred(request) => Some(request),
        }
    }
}

/// External terminals cannot hot-switch into an application window.
#[derive(Clone, Debug, Default)]
pub enum Presentation {
    #[default]
    External,
    Windowed {
        active: WindowedMode,
        workspace: Workspace,
        identity: Identity,
        pending: Transition,
        serial: u64,
    },
}
impl Presentation {
    /// Establish a usable graphical host before preparing another renderer.
    pub fn windowed() -> Self {
        Self::Windowed {
            active: WindowedMode::Gui,
            workspace: Workspace::Library,
            identity: Identity(1),
            pending: Transition::Idle,
            serial: 1,
        }
    }
    /// Logical content destination, independent of native window visibility.
    pub fn workspace(&self) -> Option<Workspace> {
        match self {
            Self::External => None,
            Self::Windowed { workspace, .. } => Some(*workspace),
        }
    }
    /// Map the main workspace for a frontend without dismissing an active surface.
    /// Settings is hosted above the graphical workspace or in the Text popover.
    pub fn project_view(&self, mode: WindowedMode, view: super::View) -> super::View {
        use super::View;
        match (mode, view) {
            (WindowedMode::Gui, View::Browse | View::Queue | View::NowPlaying | View::Settings) => {
                View::Browse
            }
            (WindowedMode::Text, View::Browse | View::Queue | View::NowPlaying) => {
                match self.workspace() {
                    Some(Workspace::NowPlaying) => View::NowPlaying,
                    _ => View::Browse,
                }
            }
            _ => view,
        }
    }
    /// Retain the workspace while a transient surface owns the current view.
    pub fn set_workspace(&mut self, value: Workspace) {
        if let Self::Windowed { workspace, .. } = self {
            *workspace = value;
        }
    }
    /// Current usable mode. An unready destination is never returned here.
    pub fn active(&self) -> Option<WindowedMode> {
        match self {
            Self::External => None,
            Self::Windowed { active, .. } => Some(*active),
        }
    }
    /// Host capability for a windowed command; external terminals have no windowed destinations.
    pub fn can_activate(&self, destination: WindowedMode) -> bool {
        self.active().is_some()
            && (destination == WindowedMode::Gui
                || cfg!(all(feature = "desktop", target_os = "macos")))
    }
    /// Identity accepted for the active adapter's input and render feedback.
    pub fn identity(&self) -> Option<Identity> {
        match self {
            Self::External => None,
            Self::Windowed { identity, .. } => Some(*identity),
        }
    }
    /// Current preparation, if a destination still needs initialization.
    pub fn pending(&self) -> Option<Preparation> {
        match self {
            Self::External => None,
            Self::Windowed { pending, .. } => pending.request(),
        }
    }
    /// Start a new ordered preparation or cancel a superseded destination.
    pub fn request(&mut self, destination: WindowedMode) -> Option<Preparation> {
        let Self::Windowed {
            active,
            pending,
            serial,
            ..
        } = self
        else {
            return None;
        };
        if destination == *active {
            *pending = Transition::Idle;
            return None;
        }
        if pending
            .request()
            .is_some_and(|request| request.destination == destination)
        {
            return pending.request();
        }
        *serial = serial.wrapping_add(1);
        let request = Preparation {
            identity: Identity(*serial),
            destination,
        };
        *pending = Transition::Preparing(request);
        Some(request)
    }
    /// Mark the current request as waiting for a platform modal to close.
    pub fn defer(&mut self, prepared: Preparation) -> bool {
        let Self::Windowed { pending, .. } = self else {
            return false;
        };
        if *pending != Transition::Preparing(prepared) {
            return false;
        }
        *pending = Transition::Deferred(prepared);
        true
    }
    /// Activate only the destination that completed the current preparation.
    pub fn commit(&mut self, prepared: Preparation) -> bool {
        let Self::Windowed {
            active,
            identity,
            pending,
            ..
        } = self
        else {
            return false;
        };
        if pending.request() != Some(prepared) {
            return false;
        }
        *active = prepared.destination;
        *identity = prepared.identity;
        *pending = Transition::Idle;
        true
    }
    /// Reject stale input, viewport changes and render feedback.
    pub fn accepts(&self, candidate: Identity) -> bool {
        matches!(self, Self::Windowed {identity,..} if *identity==candidate)
    }
    /// A failed destination leaves the existing presentation usable.
    pub fn fail(&mut self, prepared: Preparation) -> bool {
        let Self::Windowed { pending, .. } = self else {
            return false;
        };
        if pending.request() != Some(prepared) {
            return false;
        }
        *pending = Transition::Idle;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_projection_keeps_active_surfaces_over_the_workspace() {
        use super::super::View;
        let mut presentation = Presentation::windowed();
        presentation.set_workspace(Workspace::NowPlaying);
        for view in [View::Help, View::Search, View::Similar, View::Related] {
            for mode in [WindowedMode::Gui, WindowedMode::Text] {
                assert_eq!(presentation.project_view(mode, view), view);
            }
        }
        assert_eq!(
            presentation.project_view(WindowedMode::Gui, View::Settings),
            View::Browse
        );
        assert_eq!(
            presentation.project_view(WindowedMode::Text, View::Browse),
            View::NowPlaying
        );
        assert_eq!(
            presentation.project_view(WindowedMode::Gui, View::NowPlaying),
            View::Browse
        );
    }
    #[test]
    fn preparation_keeps_the_previous_mode_and_rejects_stale_completion() {
        let mut mode = Presentation::windowed();
        let abandoned = mode.request(WindowedMode::Text).unwrap();
        assert_eq!(mode.active(), Some(WindowedMode::Gui));
        mode.request(WindowedMode::Gui);
        let current = mode.request(WindowedMode::Text).unwrap();
        assert!(!mode.commit(abandoned));
        assert!(mode.commit(current));
        assert!(!mode.accepts(Identity(1)));
        assert!(mode.accepts(current.identity));
        assert_eq!(mode.active(), Some(WindowedMode::Text));
        let failed = mode.request(WindowedMode::Gui).unwrap();
        assert!(mode.fail(failed));
        assert_eq!(mode.active(), Some(WindowedMode::Text));
        assert!(Presentation::External.request(WindowedMode::Gui).is_none());
    }
}
