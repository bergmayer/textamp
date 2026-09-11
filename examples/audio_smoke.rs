//! Audio smoke test — builds a short WAV sine wave in memory, hands it to
//! the project's `RodioBackend`, and reports whether playback transitions
//! through the expected states (playing → finished).
//!
//! Run with:
//!   cargo run --release --example audio_smoke
//!
//! Silent by default; pass --audible to hear a 440 Hz tone on the default output.
//! Checks play, pause, resume, seek, and natural completion.
//! Use --direct to isolate the native output from Textamp's decoder/ring pipeline.
//! Use --stream for silent HTTP-stream seeking, pause, and cancellation checks.

use std::sync::Arc;
use std::time::{Duration, Instant};
use textamp::audio::{AudioBackend, RodioBackend};

/// Exercise the HTTP -> decoder -> device path with generated fixture audio.
/// Silent, including when --audible is supplied: this is a transport check.
async fn stream_smoke(wav: Vec<u8>) {
    use textamp::audio::{AudioEvent, AudioPlayer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn ready(rx: &mut tokio::sync::mpsc::Receiver<AudioEvent>, id: u64) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await.expect("audio event channel") {
                    AudioEvent::BufferingReady { playback_id } if playback_id == id => return,
                    AudioEvent::Error {
                        playback_id,
                        message,
                    } if playback_id == id => panic!("{message}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("stream never became ready");
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture.wav", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            if socket.read(&mut request).await.unwrap() == 0 {
                continue;
            }
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", wav.len());
            if socket.write_all(header.as_bytes()).await.is_ok() {
                let _ = socket.write_all(&wav).await;
            }
        }
    });
    let mut player = AudioPlayer::new().expect("native audio output");
    player.set_volume(0.0);
    let (events, mut receiver) = tokio::sync::mpsc::channel(32);
    player
        .play_url(
            &url,
            events,
            reqwest::Client::builder().no_proxy().build().unwrap(),
        )
        .unwrap();
    ready(&mut receiver, player.playback_id()).await;
    player.pause();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(player.is_paused());
    player.seek(Duration::from_secs(2)).unwrap();
    ready(&mut receiver, player.playback_id()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(player.is_paused(), "seek resumed paused playback");
    let position = player.position().unwrap();
    assert!(
        (Duration::from_secs(2)..Duration::from_millis(2_100)).contains(&position),
        "{position:?}"
    );
    println!("PASS: streamed forward seek preserves pause and position");

    player.resume();
    player.seek(Duration::from_millis(500)).unwrap();
    ready(&mut receiver, player.playback_id()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let position = player.position().unwrap();
    assert!(
        (Duration::from_millis(500)..Duration::from_millis(800)).contains(&position),
        "{position:?}"
    );
    println!("PASS: streamed backward seek");

    player.seek(Duration::from_millis(4_500)).unwrap();
    ready(&mut receiver, player.playback_id()).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !player.is_finished() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("seeked stream did not finish");
    assert!(player.take_failures().is_empty());
    player.seek(Duration::from_secs(1)).unwrap();
    player.stop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!player.is_playing());
    println!("PASS: streamed completion and cancellation");
    server.abort();
    let _ = server.await;
}

fn main() {
    enumerate_devices();

    let sample_rate: u32 = 44_100;
    let streaming = std::env::args().any(|arg| arg == "--stream");
    let file_backed = std::env::args().any(|arg| arg == "--file");
    let duration_secs: f32 = if streaming || file_backed { 5.0 } else { 1.0 };
    let frequency: f32 = 440.0;
    let amplitude: f32 = 0.2;

    let wav = build_sine_wav(sample_rate, duration_secs, frequency, amplitude);
    println!(
        "Generated {}-byte WAV ({} Hz, {:.2}s).",
        wav.len(),
        frequency,
        duration_secs
    );

    if streaming {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(stream_smoke(wav));
        return;
    }

    if file_backed {
        use std::io::Write;
        let mut fixture = tempfile::NamedTempFile::new().unwrap();
        fixture.write_all(&wav).unwrap();
        fixture.flush().unwrap();
        let mut player = textamp::audio::AudioPlayer::new().expect("native output");
        player.set_volume(0.0);
        player
            .play_file(textamp::library::MediaFile::local(fixture.path().into()))
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        player.pause();
        player.seek(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(player.is_paused());
        let position = player.position().unwrap();
        assert!(
            (Duration::from_secs(2)..Duration::from_millis(2100)).contains(&position),
            "{position:?}"
        );
        player.resume();
        player.seek(Duration::from_millis(4500)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !player.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(player.is_finished());
        assert!(player.take_failures().is_empty());
        player.stop();
        println!("PASS: native file-backed decode, pause, seek, completion, and stop");
        return;
    }

    if std::env::args().any(|arg| arg == "--direct") {
        let stream = rodio::OutputStreamBuilder::open_default_stream().expect("native output");
        let sink = rodio::Sink::connect_new(stream.mixer());
        sink.set_volume(0.0);
        sink.append(rodio::Decoder::new(std::io::Cursor::new(wav)).expect("WAV decoder"));
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !sink.empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !sink.empty() {
            eprintln!(
                "FAIL: direct Rodio output did not drain; position={:?}",
                sink.get_pos()
            );
            std::process::exit(5);
        }
        println!("PASS: direct Rodio output drained");
        return;
    }

    let mut backend = match RodioBackend::new() {
        Ok(b) => {
            println!("RodioBackend: opened default output stream.");
            b
        }
        Err(e) => {
            eprintln!("FAIL: RodioBackend::new: {e}");
            std::process::exit(1);
        }
    };

    backend.set_volume(if std::env::args().any(|arg| arg == "--audible") {
        0.5
    } else {
        0.0
    });

    if let Err(e) = backend.play_data(Arc::new(wav)) {
        eprintln!("FAIL: play_data: {e}");
        std::process::exit(2);
    }

    // Give the sink a moment to prime, then sample its state.
    std::thread::sleep(Duration::from_millis(50));
    println!(
        "After 50ms: is_playing={} is_paused={} is_finished={}",
        backend.is_playing(),
        backend.is_paused(),
        backend.is_finished()
    );

    if !backend.is_playing() {
        eprintln!("FAIL: backend did not enter the playing state.");
        std::process::exit(3);
    }

    backend.pause();
    std::thread::sleep(Duration::from_millis(50));
    assert!(backend.is_paused(), "pause did not take effect");
    backend.resume();
    assert!(backend.is_playing(), "resume did not take effect");
    assert!(backend.seek(Duration::from_millis(200)), "seek failed");
    println!("PASS: pause, resume, and buffered seek");
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f32(duration_secs + 1.0);
    while Instant::now() < deadline {
        if backend.is_finished() {
            let elapsed = started.elapsed().as_millis();
            println!(
                "Playback drained after {elapsed}ms (expected ~{}ms).",
                (duration_secs * 1000.0) as u32
            );
            println!("PASS: audio path is live.");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    eprintln!(
        "FAIL: sink never drained within the deadline: position={:?}, playing={}, paused={}, underruns={}",
        backend.position(), backend.is_playing(), backend.is_paused(), backend.underrun_count(),
    );
    std::process::exit(4);
}

fn enumerate_devices() {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    println!("cpal default host: {:?}", host.id());
    match host.default_output_device() {
        Some(d) => println!(
            "cpal default_output_device: {}",
            d.name().unwrap_or_else(|e| format!("<err {e}>"))
        ),
        None => println!("cpal default_output_device: None"),
    }
    match host.output_devices() {
        Ok(iter) => {
            let mut n = 0;
            for dev in iter {
                n += 1;
                println!(
                    "  output #{n}: {}",
                    dev.name().unwrap_or_else(|e| format!("<err {e}>"))
                );
            }
            if n == 0 {
                println!("  (no output devices enumerated)");
            }
        }
        Err(e) => println!("cpal output_devices() err: {e}"),
    }
}

/// Build an in-memory 16-bit mono WAV containing a sine wave.
fn build_sine_wav(sample_rate: u32, seconds: f32, freq: f32, amplitude: f32) -> Vec<u8> {
    let total_samples = (sample_rate as f32 * seconds) as u32;
    let bytes_per_sample: u16 = 2;
    let num_channels: u16 = 1;
    let byte_rate = sample_rate * num_channels as u32 * bytes_per_sample as u32;
    let block_align = num_channels * bytes_per_sample;
    let data_bytes = total_samples * bytes_per_sample as u32;

    let mut buf = Vec::with_capacity(44 + data_bytes as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&num_channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&(bytes_per_sample * 8).to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());

    let two_pi = std::f32::consts::TAU;
    for n in 0..total_samples {
        let t = n as f32 / sample_rate as f32;
        let v = (two_pi * freq * t).sin() * amplitude;
        let s = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        buf.extend_from_slice(&s.to_le_bytes());
    }

    buf
}
