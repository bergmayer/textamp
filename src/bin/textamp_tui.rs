//! Textamp's unified application entry point.
use anyhow::Result;
use textamp::app::launch::{Arguments, Mode};

fn main() -> Result<()> {
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--mtp-worker") {
        let location = std::env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("Missing USB location"))?
            .parse()?;
        std::process::exit(textamp::services::player_sync::mtp::run_worker(location));
    }
    let arguments = Arguments::parse(std::env::args().skip(1), cfg!(feature = "desktop"))?;
    if arguments.help {
        println!(
            "Textamp\n\nUsage: textamp [--gui | --text | --terminal] [--no-audio] [--verbose]\n\n--gui       Native graphical window\n--text      Embedded terminal window\n--terminal  TUI in the invoking terminal\n\nDesktop builds restore the last successful windowed mode by default."
        );
        return Ok(());
    }
    if arguments.version {
        println!("Textamp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments.mode == Mode::Terminal {
        return textamp::terminal::run(&arguments);
    }
    #[cfg(feature = "desktop")]
    {
        textamp::desktop::run_requested(arguments)
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err(textamp::app::launch::ArgumentError::DesktopUnavailable.into())
    }
}
