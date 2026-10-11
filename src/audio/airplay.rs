//! Optional Linux AirPlay sender. No library linking, pairing or privilege changes.
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirplayDestination {
    pub name: String,
    pub address: IpAddr,
    pub port: u16,
    pub airplay2: bool,
    pub txt: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct OutputDiscovery {
    pub local: Result<Vec<String>, String>,
    pub airplay: Result<Vec<AirplayDestination>, String>,
}

pub fn discover_outputs() -> Result<OutputDiscovery, String> {
    Ok(OutputDiscovery {
        local: crate::audio::local_devices(),
        airplay: discover(),
    })
}

/// Avahi's parsable format escapes separators and non-ASCII bytes as decimal octets.
fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4].iter().all(u8::is_ascii_digit)
        {
            if let Ok(number) = value[index + 1..index + 4].parse::<u8>() {
                out.push(number);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out)
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

// Informational classification only. The helper resolves the actual route from full TXT.
fn advertises_airplay2(txt: &[(String, String)]) -> bool {
    let Some((_, raw)) = txt
        .iter()
        .find(|(key, _)| key == "features")
        .or_else(|| txt.iter().find(|(key, _)| key == "ft"))
    else {
        return false;
    };
    let mut halves = raw.split(',');
    let parse =
        |value: &str| u32::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16).ok();
    let Some(low) = halves.next().and_then(parse) else {
        return false;
    };
    let high = match halves.next() {
        Some(value) => match parse(value) {
            Some(value) => value,
            None => return false,
        },
        None => 0,
    };
    if halves.next().is_some() {
        return false;
    }
    let features = (u64::from(high) << 32) | u64::from(low);
    features & ((1 << 38) | (1 << 48)) != 0
}

pub fn parse_discovery(text: &str) -> Vec<AirplayDestination> {
    let mut devices: Vec<AirplayDestination> = Vec::new();
    for line in text.lines() {
        let fields: Vec<_> = line.split(';').collect();
        if fields.len() < 9 || fields[0] != "=" || fields[2] != "IPv4" {
            continue;
        }
        let native_service = fields[4] == "_airplay._tcp";
        if !native_service && fields[4] != "_raop._tcp" {
            continue;
        }
        let Ok(address) = fields[7].parse::<IpAddr>() else {
            continue;
        };
        let Ok(port) = fields[8].parse::<u16>() else {
            continue;
        };
        if port == 0 || address.is_unspecified() || address.is_multicast() {
            continue;
        }
        let raw_name = unescape(fields[3]);
        let name = if native_service {
            raw_name
        } else {
            raw_name
                .split_once('@')
                .map(|(_, name)| name.to_owned())
                .unwrap_or(raw_name)
        };
        let txt: Vec<(String, String)> = fields
            .get(9)
            .map(|value| {
                value
                    .split('"')
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 1)
                    .filter_map(|(_, part)| {
                        unescape(part)
                            .split_once('=')
                            .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let airplay2 = advertises_airplay2(&txt);
        let device = AirplayDestination {
            name,
            address,
            port,
            airplay2,
            txt,
        };
        if let Some(existing) = devices.iter_mut().find(|d| d.address == address) {
            // Preserve both services' properties: RAOP may carry ft/sf, codecs and
            // encryption flags while the native service adds grouping/model fields.
            let mut merged = existing.txt.clone();
            for (key, value) in &device.txt {
                if let Some((_, old)) = merged.iter_mut().find(|(k, _)| k == key) {
                    *old = value.clone();
                } else {
                    merged.push((key.clone(), value.clone()));
                }
            }
            if native_service {
                *existing = device;
            }
            existing.airplay2 = advertises_airplay2(&merged);
            existing.txt = merged;
        } else {
            devices.push(device);
        }
    }
    devices.sort_by_key(|d| d.name.to_lowercase());
    devices
}

#[cfg(not(target_os = "linux"))]
pub fn discover() -> Result<Vec<AirplayDestination>, String> {
    Err("AirPlay discovery is supported on Linux".into())
}

#[cfg(target_os = "linux")]
pub fn discover() -> Result<Vec<AirplayDestination>, String> {
    let mut text = String::new();
    for service in ["_airplay._tcp", "_raop._tcp"] {
        let output = linux::capture(
            std::process::Command::new("avahi-browse").args([
                "--parsable",
                "--resolve",
                "--terminate",
                service,
            ]),
            std::time::Duration::from_secs(8),
        )
        .map_err(|e| {
            format!(
                "AirPlay discovery: {e}. avahi-browse and a running Avahi service are required."
            )
        })?;
        text.push_str(&output);
        text.push('\n');
    }
    Ok(parse_discovery(&text))
}

#[cfg(target_os = "linux")]
pub(crate) use linux::AirplayBackend;

#[cfg(target_os = "linux")]
mod linux {
    use super::AirplayDestination;
    use crate::audio::rodio_backend::{BufferedInput, PipelineEvent, RodioBackend, StreamingInput};
    use crate::audio::{AudioError, SampleTap};
    use rodio::{source::UniformSourceIterator, Source};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const RATE: u32 = 44_100;
    const FRAME_BYTES: usize = 4;
    const CHUNK_FRAMES: usize = 441;
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    const STALL_TIMEOUT: Duration = Duration::from_secs(15);

    /// Drain bounded process output while enforcing a deadline (including stalled Avahi).
    pub(super) fn capture(command: &mut Command, timeout: Duration) -> Result<String, String> {
        let mut child = OwnedChild(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?,
        );
        let mut stdout = child.0.stdout.take().ok_or("Missing process output")?;
        nonblocking(&stdout).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut buf = [0; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    if output.len() > 256 * 1024 {
                        return Err("Process output exceeded limit".into());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err(format!("Process exited with {status}"));
                }
                // Final nonblocking drain after exit.
                loop {
                    match stdout.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => output.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                return String::from_utf8(output).map_err(|_| "Process output is not UTF-8".into());
            }
            if Instant::now() >= deadline {
                return Err("Process timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn nonblocking(fd: &impl AsRawFd) -> std::io::Result<()> {
        // SAFETY: the descriptor is borrowed from a live owned file/pipe.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    #[derive(Default)]
    struct Timeline {
        position: Duration,
        started: bool,
        finished: bool,
        playing: bool,
    }
    struct Shared {
        cancelled: Arc<AtomicBool>,
        paused: AtomicBool,
        volume: AtomicU32,
        timeline: Mutex<Timeline>,
    }
    enum Input {
        Buffered(BufferedInput, Duration),
        Stream(StreamingInput),
    }
    enum Pcm {
        Bytes(Vec<u8>),
        End,
        Error,
    }

    pub(crate) struct AirplayBackend {
        destination: AirplayDestination,
        helper: PathBuf,
        volume: f32,
        paused: bool,
        shared: Option<Arc<Shared>>,
        worker: Option<JoinHandle<()>>,
        events: Option<mpsc::Receiver<PipelineEvent>>,
        current_data: Option<BufferedInput>,
        current_id: u64,
    }
    impl AirplayBackend {
        pub(crate) fn new(destination: AirplayDestination) -> Result<Self, AudioError> {
            let adjacent = std::env::current_exe()
                .ok()
                .and_then(|path| {
                    path.parent()
                        .map(|parent| parent.join("textamp-airplay/cliairplay"))
                })
                .filter(|path| path.is_file());
            let candidate = std::env::var_os("TEXTAMP_AIRPLAY_HELPER")
                .or_else(|| adjacent.map(|path| path.into_os_string()))
                .unwrap_or_else(|| "cliairplay".into());
            let helper = if PathBuf::from(&candidate).is_absolute() {
                PathBuf::from(candidate)
            } else {
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .map(|p| p.join(&candidate)).find(|p| p.is_file())
                    .ok_or_else(|| AudioError::PlaybackError("AirPlay sender not found. Install music-assistant/airplay-cli separately and set TEXTAMP_AIRPLAY_HELPER to its executable.".into()))?
            };
            let check = capture(Command::new(&helper).arg("--check"), Duration::from_secs(3))
                .map_err(AudioError::PlaybackError)?;
            if !check.trim().starts_with("cliairplay ") || !check.trim().ends_with(" check") {
                return Err(AudioError::PlaybackError(
                    "Incompatible AirPlay sender: expected current music-assistant/airplay-cli"
                        .into(),
                ));
            }
            Ok(Self {
                destination,
                helper,
                volume: 0.8,
                paused: false,
                shared: None,
                worker: None,
                events: None,
                current_data: None,
                current_id: 0,
            })
        }
        fn start(&mut self, id: u64, input: Input, position: Duration) -> Result<(), AudioError> {
            self.stop_worker();
            let shared = Arc::new(Shared {
                cancelled: Arc::new(AtomicBool::new(false)),
                paused: AtomicBool::new(self.paused),
                volume: AtomicU32::new(self.volume.to_bits()),
                timeline: Mutex::new(Timeline {
                    position,
                    ..Default::default()
                }),
            });
            let worker_shared = shared.clone();
            let destination = self.destination.clone();
            let helper = self.helper.clone();
            let (tx, rx) = mpsc::sync_channel(4);
            self.worker = Some(
                std::thread::Builder::new()
                    .name("textamp-airplay".into())
                    .spawn(move || {
                        let outcome = crate::util::catch_expected_panic(|| {
                            run(&helper, &destination, input, &worker_shared, id, &tx)
                        });
                        let failed = !matches!(outcome, Ok(Ok(())));
                        if failed {
                            // Sender diagnostics can contain pairing material; report only bounded generic errors.
                            let message = match outcome {
                                Ok(Err(message)) => message,
                                _ => "AirPlay worker failed".into(),
                            };
                            let _ = tx.try_send(PipelineEvent::Failed {
                                playback_id: id,
                                message,
                            });
                        }
                        worker_shared.cancelled.store(true, Ordering::Release);
                    })
                    .map_err(|e| AudioError::PlaybackError(e.to_string()))?,
            );
            self.events = Some(rx);
            self.shared = Some(shared);
            self.current_id = id;
            Ok(())
        }
        pub(crate) fn start_buffered(
            &mut self,
            id: u64,
            data: BufferedInput,
            position: Duration,
        ) -> Result<(), AudioError> {
            self.current_data = Some(data.clone());
            self.start(id, Input::Buffered(data, position), position)
        }
        pub(crate) fn start_stream(
            &mut self,
            id: u64,
            input: StreamingInput,
        ) -> Result<(), AudioError> {
            let position = input.start_position;
            self.current_data = None;
            self.start(id, Input::Stream(input), position)
        }
        fn stop_worker(&mut self) {
            if let Some(shared) = self.shared.take() {
                shared.cancelled.store(true, Ordering::Release);
            }
            if let Some(worker) = self.worker.take() {
                crate::audio::finish_thread(worker, "AirPlay sender", Duration::from_secs(1));
            }
            self.events = None;
        }
        pub(crate) fn poll_pipeline(&mut self, paused: bool) -> Vec<PipelineEvent> {
            self.paused = paused;
            if let Some(shared) = &self.shared {
                shared.paused.store(paused, Ordering::Release);
            }
            self.events
                .as_ref()
                .map(|rx| rx.try_iter().collect())
                .unwrap_or_default()
        }
        pub(crate) fn sample_tap(&self) -> SampleTap {
            SampleTap::default()
        }
        pub(crate) fn underrun_count(&self) -> u64 {
            0
        }
        pub(crate) fn volume(&self) -> f32 {
            self.volume
        }
        pub(crate) fn set_volume(&mut self, volume: f32) {
            self.volume = if volume.is_finite() {
                volume.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if let Some(shared) = &self.shared {
                shared
                    .volume
                    .store(self.volume.to_bits(), Ordering::Release);
            }
        }
        pub(crate) fn pause(&mut self) {
            self.paused = true;
            if let Some(shared) = &self.shared {
                shared.paused.store(true, Ordering::Release);
            }
        }
        pub(crate) fn resume(&mut self) {
            self.paused = false;
            if let Some(shared) = &self.shared {
                shared.paused.store(false, Ordering::Release);
            }
        }
        pub(crate) fn stop(&mut self) {
            self.stop_worker();
            self.current_data = None;
            self.paused = false;
        }
        pub(crate) fn seek(&mut self, position: Duration) -> bool {
            self.current_data
                .clone()
                .is_some_and(|data| self.start_buffered(self.current_id, data, position).is_ok())
        }
        pub(crate) fn position(&self) -> Option<Duration> {
            self.shared
                .as_ref()
                .map(|s| crate::audio::lock_or_recover(&s.timeline).position)
        }
        pub(crate) fn is_finished(&self) -> bool {
            self.shared
                .as_ref()
                .is_some_and(|s| crate::audio::lock_or_recover(&s.timeline).finished)
        }
        pub(crate) fn is_paused(&self) -> bool {
            self.shared.as_ref().is_some_and(|s| {
                s.paused.load(Ordering::Acquire)
                    && !s.cancelled.load(Ordering::Acquire)
                    && crate::audio::lock_or_recover(&s.timeline).started
            })
        }
        pub(crate) fn is_playing(&self) -> bool {
            self.shared.as_ref().is_some_and(|s| {
                let t = crate::audio::lock_or_recover(&s.timeline);
                t.playing && !t.finished && !s.cancelled.load(Ordering::Acquire)
            })
        }
    }
    impl Drop for AirplayBackend {
        fn drop(&mut self) {
            self.stop_worker();
        }
    }

    fn decode(input: Input, cancelled: Arc<AtomicBool>, tx: mpsc::SyncSender<Pcm>) {
        let outcome = crate::util::catch_expected_panic(|| -> Result<(), AudioError> {
            let failure = match &input {
                Input::Stream(input) => Some(input.failure.clone()),
                _ => None,
            };
            let decoder: Box<dyn Source<Item = f32> + Send> = match input {
                Input::Buffered(data, position) => {
                    let mut decoder = RodioBackend::try_decode(data)?;
                    if !position.is_zero() {
                        decoder
                            .try_seek(position)
                            .map_err(|e| AudioError::SeekError(e.to_string()))?;
                    }
                    Box::new(decoder)
                }
                Input::Stream(input) => {
                    let position = input.start_position;
                    let mut decoder = RodioBackend::try_decode_stream(input, cancelled.clone())?;
                    crate::audio::rodio_backend::skip_stream_to(&mut decoder, position, &cancelled);
                    Box::new(decoder)
                }
            };
            let mut source = UniformSourceIterator::new(decoder, 2, RATE);
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return Ok(());
                }
                let bytes: Vec<_> = source
                    .by_ref()
                    .take(CHUNK_FRAMES * 2)
                    .flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
                    .collect();
                if bytes.is_empty() {
                    if failure.as_ref().and_then(|f| f.message()).is_some() {
                        return Err(AudioError::PlaybackError("Source transfer failed".into()));
                    }
                    send_pcm(&tx, Pcm::End, &cancelled);
                    return Ok(());
                }
                if !send_pcm(&tx, Pcm::Bytes(bytes), &cancelled) {
                    return Ok(());
                }
            }
        });
        if !matches!(outcome, Ok(Ok(()))) {
            send_pcm(&tx, Pcm::Error, &cancelled);
        }
    }
    fn send_pcm(tx: &mpsc::SyncSender<Pcm>, mut value: Pcm, cancelled: &AtomicBool) -> bool {
        loop {
            if cancelled.load(Ordering::Acquire) {
                return false;
            }
            match tx.try_send(value) {
                Ok(()) => return true,
                Err(mpsc::TrySendError::Disconnected(_)) => return false,
                Err(mpsc::TrySendError::Full(v)) => {
                    value = v;
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// Accept status lines only; discard non-status diagnostics and credential output.
    fn status_lines(reader: &mut impl Read, pending: &mut Vec<u8>) -> Result<Vec<String>, String> {
        let mut buf = [0; 4096];
        let mut lines = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(end) = pending.iter().position(|v| *v == b'\n') {
                        let line = String::from_utf8_lossy(&pending[..end]).to_string();
                        pending.drain(..=end);
                        if line.starts_with("[STATUS] ") {
                            lines.push(line);
                        }
                    }
                    if pending.len() > 8192 {
                        return Err("AirPlay sender status exceeded limit".into());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return Err("Could not read AirPlay sender status".into()),
            }
        }
        Ok(lines)
    }
    fn command(pipe: &mut File, value: &str) -> Result<(), String> {
        pipe.write_all(value.as_bytes())
            .map_err(|_| "AirPlay command pipe unavailable".into())
    }
    fn epoch_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
    fn start_epoch(line: &str) -> Option<u64> {
        if !line.starts_with("[STATUS] started ") {
            return None;
        }
        line.split_whitespace()
            .find_map(|v| v.strip_prefix("at_unix_ms=")?.parse().ok())
    }

    fn run(
        helper: &std::path::Path,
        destination: &AirplayDestination,
        input: Input,
        shared: &Shared,
        id: u64,
        events: &mpsc::SyncSender<PipelineEvent>,
    ) -> Result<(), String> {
        let directory = tempfile::tempdir()
            .map_err(|_| "Could not create private AirPlay runtime directory")?;
        let fifo = directory.path().join("commands");
        let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())
            .map_err(|_| "Invalid runtime path")?;
        // SAFETY: CString remains alive and FIFO is private to this session.
        if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
            return Err("Could not create AirPlay command pipe".into());
        }
        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .map_err(|_| "Could not open AirPlay command pipe")?;
        let mut cmd = Command::new(helper);
        cmd.args([
            "--timing",
            "ntp",
            "--samplerate",
            "44100",
            "--channels",
            "2",
            "--bitdepth",
            "16",
            "--port",
        ])
        .arg(destination.port.to_string())
        .arg("--volume")
        .arg((shared_volume(shared) * 100.0).round().to_string())
        .arg("--cmdpipe")
        .arg(&fifo)
        .arg("--debug")
        .arg("0");
        sender_destination_args(&mut cmd, destination);
        let mut child = OwnedChild(
            cmd.arg(destination.address.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|_| "Could not launch AirPlay sender. Check TEXTAMP_AIRPLAY_HELPER.")?,
        );
        let mut stdin = child.0.stdin.take();
        let mut stdout = child.0.stdout.take().ok_or("Missing AirPlay stdout")?;
        let mut stderr = child.0.stderr.take().ok_or("Missing AirPlay stderr")?;
        nonblocking(stdin.as_ref().ok_or("Missing AirPlay stdin")?)
            .map_err(|_| "Could not configure PCM pipe")?;
        nonblocking(&stdout).map_err(|_| "Could not configure sender status")?;
        nonblocking(&stderr).map_err(|_| "Could not configure sender status")?;
        let (pcm_tx, pcm_rx) = mpsc::sync_channel(32);
        let cancelled = shared.cancelled.clone();
        let decoder = std::thread::Builder::new()
            .name("textamp-airplay-decoder".into())
            .spawn(move || decode(input, cancelled, pcm_tx))
            .map_err(|_| "Could not start AirPlay decoder")?;
        let result = pump(
            &mut child,
            &mut stdin,
            &mut stdout,
            &mut stderr,
            &mut pipe,
            &pcm_rx,
            shared,
            id,
            events,
        );
        shared.cancelled.store(true, Ordering::Release);
        let _ = command(&mut pipe, "ACTION=STOP\n");
        drop(child);
        crate::audio::finish_thread(decoder, "AirPlay decoder", Duration::from_millis(500));
        result
    }
    fn sender_destination_args(cmd: &mut Command, destination: &AirplayDestination) {
        // _raop._tcp can advertise native AirPlay 2. Let the pinned helper
        // resolve protocol/auth from features/ft and flags/sf, never the service name.
        cmd.args(["--protocol", "auto", "--txt"]).arg(
            destination
                .txt
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        // Preserve legacy/compat properties alongside the complete TXT records.
        for key in ["et", "md", "am", "pk", "pw", "cn"] {
            if let Some((_, value)) = destination.txt.iter().find(|(k, _)| k == key) {
                cmd.arg(format!("--{key}")).arg(value);
            }
        }
    }

    fn shared_volume(shared: &Shared) -> f32 {
        f32::from_bits(shared.volume.load(Ordering::Acquire))
    }

    #[allow(clippy::too_many_arguments)]
    fn pump(
        child: &mut OwnedChild,
        stdin: &mut Option<std::process::ChildStdin>,
        stdout: &mut impl Read,
        stderr: &mut impl Read,
        pipe: &mut File,
        pcm: &mpsc::Receiver<Pcm>,
        shared: &Shared,
        id: u64,
        events: &mpsc::SyncSender<PipelineEvent>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let base = crate::audio::lock_or_recover(&shared.timeline).position;
        let mut out_buf = Vec::new();
        let mut err_buf = Vec::new();
        let mut bytes = Vec::new();
        let mut offset = 0;
        let mut written: u64 = 0;
        let mut start = None;
        let mut start_requested = false;
        let mut paused = false;
        let mut pause_at = None;
        let mut pause_total = Duration::ZERO;
        let mut volume = shared_volume(shared);
        let mut ended = false;
        let mut eof = false;
        let mut last_progress = Instant::now();
        loop {
            if shared.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            for line in status_lines(stdout, &mut out_buf)?
                .into_iter()
                .chain(status_lines(stderr, &mut err_buf)?)
            {
                if line.starts_with("[STATUS] audio ") && !start_requested {
                    command(pipe, "START_UNIX_MS=0\nACTION=START\n")?;
                    start_requested = true;
                }
                if let Some(epoch) = start_epoch(&line) {
                    start = Some(
                        Instant::now() + Duration::from_millis(epoch.saturating_sub(epoch_ms())),
                    );
                    crate::audio::lock_or_recover(&shared.timeline).started = true;
                    events
                        .try_send(PipelineEvent::Ready { playback_id: id })
                        .map_err(|_| "AirPlay event queue unavailable")?;
                }
                if line.starts_with("[STATUS] eof") {
                    eof = true;
                }
                if line.starts_with("[STATUS] clock_ready") && line.contains("state=stalled") {
                    return Err("AirPlay receiver clock stalled. This output uses unprivileged NTP timing; this receiver may require approved PTP setup.".into());
                }
            }
            let desired_pause = shared.paused.load(Ordering::Acquire);
            if start.is_some() && desired_pause != paused {
                command(
                    pipe,
                    if desired_pause {
                        "ACTION=PAUSE\n"
                    } else {
                        "ACTION=PLAY\n"
                    },
                )?;
                if desired_pause {
                    pause_at = Some(Instant::now());
                } else if let Some(at) = pause_at.take() {
                    pause_total += at.elapsed();
                }
                paused = desired_pause;
                last_progress = Instant::now();
            }
            let desired_volume = shared_volume(shared);
            if desired_volume.to_bits() != volume.to_bits() {
                command(
                    pipe,
                    &format!("VOLUME={}\n", (desired_volume * 100.0).round()),
                )?;
                volume = desired_volume;
            }
            let elapsed = start
                .map(|s| {
                    pause_at
                        .unwrap_or_else(Instant::now)
                        .saturating_duration_since(s)
                        .saturating_sub(pause_total)
                })
                .unwrap_or_default();
            let fed = Duration::from_secs_f64(written as f64 / (RATE as f64 * FRAME_BYTES as f64));
            {
                let mut timeline = crate::audio::lock_or_recover(&shared.timeline);
                timeline.position = base + elapsed.min(fed);
                timeline.playing = start.is_some() && !paused;
                if ended && eof && start.is_some() && elapsed >= fed {
                    timeline.finished = true;
                    timeline.playing = false;
                    return Ok(());
                }
            }
            if let Some(status) = child.0.try_wait().map_err(|_| "Could not poll sender")? {
                if !status.success() || !ended || !eof {
                    return Err("AirPlay sender disconnected or rejected the receiver. Check receiver access/pairing requirements; Textamp does not pair automatically.".into());
                }
                if !paused && start.is_some() && elapsed >= fed {
                    crate::audio::lock_or_recover(&shared.timeline).finished = true;
                    return Ok(());
                }
            }
            if start.is_none() && Instant::now() >= deadline {
                return Err("AirPlay connection/start timed out. Check receiver access and whether pairing or PTP timing is required.".into());
            }
            if !paused && !ended && fed <= elapsed + Duration::from_millis(750) {
                if offset == bytes.len() {
                    match pcm.try_recv() {
                        Ok(Pcm::Bytes(next)) => {
                            bytes = next;
                            offset = 0;
                        }
                        Ok(Pcm::End) => {
                            ended = true;
                            stdin.take();
                        }
                        Ok(Pcm::Error) | Err(mpsc::TryRecvError::Disconnected) => {
                            return Err("AirPlay audio decoding or source transfer failed".into())
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                }
                if let Some(input) = stdin.as_mut().filter(|_| offset < bytes.len()) {
                    match input.write(&bytes[offset..]) {
                        Ok(0) => return Err("AirPlay PCM input closed".into()),
                        Ok(n) => {
                            offset += n;
                            written += n as u64;
                            last_progress = Instant::now();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => return Err("AirPlay sender stopped accepting audio".into()),
                    }
                }
            }
            if !paused && !ended && last_progress.elapsed() > STALL_TIMEOUT {
                return Err("AirPlay audio transfer stalled".into());
            }
            if ended && !paused && elapsed > fed + Duration::from_secs(5) {
                return Err("AirPlay sender did not acknowledge the end of audio".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        include!("airplay_mock_tests.rs");
        #[test]
        fn auto_route_receives_all_recorded_and_malformed_properties() {
            let mut devices =
                super::super::parse_discovery(include_str!("receiver_advertisements.txt"));
            for txt in [
                vec![],
                vec![
                    ("ft".into(), "malformed".into()),
                    ("sf".into(), "bad".into()),
                ],
                vec![("features".into(), "0x0".into())],
            ] {
                let mut device = devices[0].clone();
                device.txt = txt;
                device.airplay2 = false;
                devices.push(device);
            }
            for destination in devices {
                let mut cmd = Command::new("mock");
                sender_destination_args(&mut cmd, &destination);
                let args: Vec<_> = cmd
                    .get_args()
                    .map(|value| value.to_str().unwrap())
                    .collect();
                assert_eq!(&args[..3], ["--protocol", "auto", "--txt"]);
                assert_eq!(
                    args[3],
                    destination
                        .txt
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                for key in ["et", "md", "am", "pk", "pw", "cn"] {
                    if let Some((_, value)) = destination.txt.iter().find(|(k, _)| k == key) {
                        let flag = format!("--{key}");
                        let index = args.iter().position(|v| *v == flag).unwrap();
                        assert_eq!(args[index + 1], value);
                    }
                }
                assert!(!args.iter().any(|arg| [
                    "--auth",
                    "--pair-setup",
                    "--ptp",
                    "--ap2-native"
                ]
                .contains(arg)));
            }
        }
        #[test]
        fn status_does_not_forward_credentials() {
            let mut input =
                std::io::Cursor::new(b"CREDENTIALS: secret\n[STATUS] started at_unix_ms=123\n");
            let lines = status_lines(&mut input, &mut Vec::new()).unwrap();
            assert_eq!(lines, ["[STATUS] started at_unix_ms=123"]);
            assert_eq!(start_epoch(&lines[0]), Some(123));
        }
        #[test]
        fn timeout_reaps_process() {
            assert!(capture(
                Command::new("sh").args(["-c", "exec sleep 20"]),
                Duration::from_millis(30)
            )
            .unwrap_err()
            .contains("timed out"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_prefers_airplay2_and_unescapes_names() {
        let lines = "=;eth0;IPv4;AABB@Living\\032Room;_raop._tcp;local;room.local;192.168.1.5;5000;\"cn=0,1\"\n=;eth0;IPv4;Living\\032Room;_airplay._tcp;local;room.local;192.168.1.5;7000;\"features=0x4A7FCA00,0x3C354BD0\" \"model=AudioAccessory5,1\"";
        let devices = parse_discovery(lines);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Living Room");
        assert!(devices[0].airplay2);
        assert_eq!(devices[0].port, 7000);
        assert_eq!(
            devices[0]
                .txt
                .iter()
                .find(|(key, _)| key == "model")
                .unwrap(),
            &("model".into(), "AudioAccessory5,1".into())
        );
    }
    #[test]
    fn invalid_records_cannot_become_sender_arguments() {
        assert!(parse_discovery(
            "=;eth0;IPv4;Oops;_airplay._tcp;local;host;--pair-setup;7000;\"features=x\""
        )
        .is_empty());
        assert!(parse_discovery(
            "=;eth0;IPv4;Oops;_airplay._tcp;local;host;0.0.0.0;7000;\"features=x\""
        )
        .is_empty());
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    #[test]
    fn recorded_raop_advertisements_are_airplay2_without_a_native_service() {
        let devices = parse_discovery(include_str!("receiver_advertisements.txt"));
        assert_eq!(devices.len(), 6);
        assert!(devices
            .iter()
            .all(|device| device.airplay2 && device.port == 7000));
        let left = devices.iter().find(|device| device.name == "left").unwrap();
        assert!(left
            .txt
            .contains(&("ft".into(), "0x4A7FCA00,0x3C354BD0".into())));
        assert!(left.txt.contains(&("sf".into(), "0xb8c04".into())));
        let tv = devices
            .iter()
            .find(|device| device.name == "telly")
            .unwrap();
        assert!(tv.txt.contains(&("sf".into(), "0x18644".into())));
    }
    #[test]
    fn missing_or_malformed_features_never_invent_airplay2() {
        for raw in [
            "",
            "garbage",
            "0xGG,0x0",
            "0x0,0x100000000",
            "0x0,0x10000,0x0",
            "0x123",
        ] {
            assert!(
                !advertises_airplay2(&[("features".into(), raw.into())]),
                "{raw}"
            );
        }
        assert!(!advertises_airplay2(&[]));
        assert!(advertises_airplay2(&[("ft".into(), "0x0,0x10000".into())]));
        // PIN/status flags alone are not an AirPlay 2 protocol advertisement.
        assert!(!advertises_airplay2(&[("sf".into(), "0x200".into())]));
    }
    #[test]
    fn duplicate_services_preserve_full_txt_in_either_order() {
        let raop = "=;eth0;IPv4;AABB@Living\\032Room;_raop._tcp;local;room.local;192.168.1.5;7000;\"ft=0x0,0x10000\" \"sf=0x204\" \"cn=0,1\"";
        let native = "=;eth0;IPv4;Living\\032Room;_airplay._tcp;local;room.local;192.168.1.5;7000;\"features=0x0,0x10000\" \"model=AudioAccessory5,1\"";
        for lines in [format!("{raop}\n{native}"), format!("{native}\n{raop}")] {
            let devices = parse_discovery(&lines);
            assert_eq!(devices.len(), 1);
            assert!(devices[0].airplay2);
            assert_eq!(devices[0].name, "Living Room");
            for key in ["ft", "sf", "cn", "features", "model"] {
                assert!(devices[0].txt.iter().any(|(k, _)| k == key));
            }
        }
    }
}
