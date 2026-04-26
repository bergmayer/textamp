//! Audio smoke test — builds a short WAV sine wave in memory, hands it to
//! the project's `RodioBackend`, and reports whether playback transitions
//! through the expected states (playing → finished).
//!
//! Run with:
//!   cargo run --release --example audio_smoke
//!
//! Output is success/failure text on stdout; a 440 Hz tone should also be
//! audible on the default output.

use std::sync::Arc;
use std::time::{Duration, Instant};
use textamp::audio::{AudioBackend, RodioBackend};

fn main() {
    enumerate_devices();

    let sample_rate: u32 = 44_100;
    let duration_secs: f32 = 1.0;
    let frequency: f32 = 440.0;
    let amplitude: f32 = 0.2;

    let wav = build_sine_wav(sample_rate, duration_secs, frequency, amplitude);
    println!("Generated {}-byte WAV ({} Hz, {:.2}s).", wav.len(), frequency, duration_secs);

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

    backend.set_volume(0.5);

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

    let started = Instant::now();
    let deadline = started + Duration::from_secs_f32(duration_secs + 1.0);
    while Instant::now() < deadline {
        if backend.is_finished() {
            let elapsed = started.elapsed().as_millis();
            println!("Playback drained after {elapsed}ms (expected ~{}ms).", (duration_secs * 1000.0) as u32);
            println!("PASS: audio path is live.");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    eprintln!("FAIL: sink never drained within the deadline.");
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
    buf.extend_from_slice(&16u32.to_le_bytes());       // PCM fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes());        // PCM format
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
