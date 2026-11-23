//! Launch choices and windowed presentation identity.
use serde::{Deserialize, Serialize};

/// Windowed presentation remembered after successful activation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowedMode {
    #[default]
    Gui,
    Text,
}
/// User-facing application modes; saved GUI arrangement remains compatible with existing profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationMode {
    Text,
    GuiSingle,
    GuiMultiple,
}
impl ApplicationMode {
    /// Choices exposed by a windowed Settings host.
    pub const ALL: [Self; 3] = [Self::GuiMultiple, Self::GuiSingle, Self::Text];
    /// Canonical setting label shared by the frontends.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::GuiSingle => "GUI: Single Window",
            Self::GuiMultiple => "GUI: Multiple Windows",
        }
    }
    /// Next mode in the application-window cycle.
    pub const fn next(self) -> Self {
        match self {
            Self::GuiMultiple => Self::GuiSingle,
            Self::GuiSingle => Self::Text,
            Self::Text => Self::GuiMultiple,
        }
    }
    /// Renderer required by this choice, independent of GUI window arrangement.
    pub const fn presentation(self) -> WindowedMode {
        match self {
            Self::Text => WindowedMode::Text,
            Self::GuiSingle | Self::GuiMultiple => WindowedMode::Gui,
        }
    }
    /// GUI arrangement to apply; Text retains the remembered GUI arrangement.
    pub const fn gui_multiple_windows(self) -> Option<bool> {
        match self {
            Self::Text => None,
            Self::GuiSingle => Some(false),
            Self::GuiMultiple => Some(true),
        }
    }
}

/// Requested process entry path; Restore reads the saved windowed choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Restore,
    Windowed(WindowedMode),
    Terminal,
}

/// Validated arguments, before configuration, audio or window initialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arguments {
    pub mode: Mode,
    pub verbose: bool,
    pub no_audio: bool,
    pub help: bool,
    pub version: bool,
}

/// Invalid launch flags do not acquire a profile or create an application window.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArgumentError {
    #[error("Choose only one launch mode: --gui, --text, or --terminal")]
    ContradictoryModes,
    #[error("Unknown argument: {0}")]
    Unknown(String),
    #[error("Windowed modes require a desktop-enabled Textamp build")]
    DesktopUnavailable,
    #[error("--terminal requires terminal input and output; run it from an interactive shell")]
    TerminalUnavailable,
}

impl Arguments {
    /// Parse explicit launch flags without probing the host or loading settings.
    /// # Errors
    /// Rejects unknown arguments and different mode flags in the same invocation.
    pub fn parse(
        args: impl IntoIterator<Item = String>,
        desktop: bool,
    ) -> Result<Self, ArgumentError> {
        let mut result = Self {
            mode: if desktop {
                Mode::Restore
            } else {
                Mode::Terminal
            },
            verbose: false,
            no_audio: false,
            help: false,
            version: false,
        };
        let mut selected = None;
        for arg in args {
            let mode = match arg.as_str() {
                "--gui" => Some(Mode::Windowed(WindowedMode::Gui)),
                "--text" => Some(Mode::Windowed(WindowedMode::Text)),
                "--terminal" => Some(Mode::Terminal),
                "--verbose" | "-v" => {
                    result.verbose = true;
                    None
                }
                "--no-audio" => {
                    result.no_audio = true;
                    None
                }
                "--help" | "-h" => {
                    result.help = true;
                    None
                }
                "--version" | "-V" => {
                    result.version = true;
                    None
                }
                _ => return Err(ArgumentError::Unknown(arg)),
            };
            if let Some(mode) = mode {
                if selected.is_some_and(|previous| previous != mode) {
                    return Err(ArgumentError::ContradictoryModes);
                }
                selected = Some(mode);
                result.mode = mode;
            }
        }
        if !desktop && matches!(result.mode, Mode::Windowed(_)) {
            return Err(ArgumentError::DesktopUnavailable);
        }
        Ok(result)
    }
    /// Enforce external-terminal suitability before terminal setup or runtime startup.
    /// # Errors
    /// Rejects redirected input or output for --terminal.
    pub fn validate_terminal(&self, input: bool, output: bool) -> Result<(), ArgumentError> {
        if !(self.mode != Mode::Terminal || self.help || self.version || input && output) {
            Err(ArgumentError::TerminalUnavailable)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(flags: &[&str], desktop: bool) -> Result<Arguments, ArgumentError> {
        Arguments::parse(flags.iter().map(|flag| (*flag).into()), desktop)
    }
    #[test]
    fn modes_are_exclusive_and_default_depends_on_the_build() {
        assert_eq!(parse(&[], true).unwrap().mode, Mode::Restore);
        assert_eq!(parse(&[], false).unwrap().mode, Mode::Terminal);
        assert_eq!(
            parse(&["--gui", "--terminal"], true),
            Err(ArgumentError::ContradictoryModes)
        );
        assert_eq!(
            parse(&["--text"], false),
            Err(ArgumentError::DesktopUnavailable)
        );
        assert_eq!(
            parse(&["--terminal"], true)
                .unwrap()
                .validate_terminal(false, true),
            Err(ArgumentError::TerminalUnavailable)
        );
    }
}
