use std::os::unix::fs::PermissionsExt;

fn fixture(fail: bool) -> (tempfile::TempDir, AirplayBackend) {
    let root = tempfile::tempdir().unwrap();
    let helper = root.path().join("cliairplay");
    let script = r#"#!/usr/bin/python3
import sys, os, select, time
from pathlib import Path
if '--check' in sys.argv:
    print('cliairplay mock check'); sys.exit(0)
if FAIL:
    print('CREDENTIALS: must-never-be-reported', file=sys.stderr)
    sys.exit(2)
pipe = sys.argv[sys.argv.index('--cmdpipe') + 1]
fd = os.open(pipe, os.O_RDONLY | os.O_NONBLOCK)
os.set_blocking(0, False)
pending = b''
ready = False
root = Path(__file__).parent
while True:
    readable, _, _ = select.select([0, fd], [], [], .02)
    if 0 in readable:
        data = os.read(0, 65536)
        if not data:
            print('[STATUS] eof', file=sys.stderr, flush=True)
            sys.exit(0)
        with open(root / 'pcm', 'ab') as f: f.write(data)
        if not ready:
            print('[STATUS] audio buffered_ms=10', file=sys.stderr, flush=True)
            ready = True
    if fd in readable:
        pending += os.read(fd, 4096)
        while b'\n' in pending:
            line, pending = pending.split(b'\n', 1)
            with open(root / 'commands', 'ab') as f: f.write(line + b'\n')
            if line == b'ACTION=START':
                print('[STATUS] started requested_unix_ms=0 at_unix_ms=' + str(int(time.time()*1000)+200), file=sys.stderr, flush=True)
            if line == b'ACTION=STOP': sys.exit(0)
"#.replace("FAIL", if fail { "True" } else { "False" });
    std::fs::write(&helper, script).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let backend = AirplayBackend {
        destination: AirplayDestination { name: "Mock".into(), address: "127.0.0.1".parse().unwrap(), port: 7000, airplay2: true, txt: vec![] },
        helper, volume: 0.8, paused: false, shared: None, worker: None, events: None, current_data: None, current_id: 0,
    };
    (root, backend)
}
fn wav(seconds: u32) -> BufferedInput {
    // Mono 22.05 kHz input exercises channel conversion and resampling.
    let size = seconds * 22_050 * 2;
    let mut bytes = b"RIFF".to_vec(); bytes.extend((36 + size).to_le_bytes()); bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes()); bytes.extend(1_u16.to_le_bytes()); bytes.extend(1_u16.to_le_bytes());
    bytes.extend(22_050_u32.to_le_bytes()); bytes.extend(44_100_u32.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes()); bytes.extend(16_u16.to_le_bytes()); bytes.extend(b"data"); bytes.extend(size.to_le_bytes());
    for _ in 0..seconds * 22_050 { bytes.extend(8_192_i16.to_le_bytes()); }
    BufferedInput::Bytes(Arc::new(bytes))
}
fn wait_started(backend: &mut AirplayBackend) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let events = backend.poll_pipeline(false);
        assert!(!events.iter().any(|e| matches!(e, PipelineEvent::Failed { .. })));
        if events.iter().any(|e| matches!(e, PipelineEvent::Ready { .. })) { return; }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("mock sender did not start");
}
#[test] fn pcm_transport_controls_and_stop() {
    let (root, mut backend) = fixture(false);
    backend.start_buffered(7, wav(2), Duration::ZERO).unwrap();
    wait_started(&mut backend);
    std::thread::sleep(Duration::from_millis(250));
    backend.pause(); backend.set_volume(0.25);
    std::thread::sleep(Duration::from_millis(50));
    let paused = backend.position().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(backend.position().unwrap(), paused);
    backend.resume();
    std::thread::sleep(Duration::from_millis(50));
    let commands = std::fs::read_to_string(root.path().join("commands")).unwrap();
    assert!(commands.contains("ACTION=PAUSE") && commands.contains("ACTION=PLAY") && commands.contains("VOLUME=25"));
    let pcm = std::fs::read(root.path().join("pcm")).unwrap();
    assert!(pcm.len() >= 441 * 4 && pcm.len() % 4 == 0);
    assert_eq!(&pcm[0..2], &pcm[2..4]);
    assert!(backend.seek(Duration::from_secs(1)));
    wait_started(&mut backend);
    assert!(backend.position().unwrap() >= Duration::from_secs(1));
    let now = Instant::now(); backend.stop();
    assert!(now.elapsed() < Duration::from_secs(1));
    assert!(backend.shared.is_none());
}
#[test] fn sender_failure_is_not_natural_completion_or_secret_output() {
    let (_root, mut backend) = fixture(true);
    backend.start_buffered(42, wav(1), Duration::ZERO).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let failures = backend.poll_pipeline(false);
        if let Some(PipelineEvent::Failed { playback_id, message }) = failures.first() {
            assert_eq!(*playback_id, 42); assert!(!message.contains("must-never")); assert!(!backend.is_finished()); break;
        }
        assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10));
    }
}
#[test] fn end_waits_for_the_audible_timeline() {
    let (_root, mut backend) = fixture(false);
    backend.start_buffered(8, wav(1), Duration::ZERO).unwrap();
    wait_started(&mut backend);
    assert!(!backend.is_finished());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !backend.is_finished() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
    assert!(backend.position().unwrap() >= Duration::from_millis(990));
}

#[test]
fn stopping_a_stalled_stream_cancels_decoder_and_sender() {
    let (_root, mut backend) = fixture(false);
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let input = StreamingInput::new(rx, crate::audio::rodio_backend::StreamFailure::default(), None, Duration::ZERO);
    backend.start_stream(19, input).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let now = Instant::now(); backend.stop();
    assert!(now.elapsed() < Duration::from_secs(1));
}

#[test]
fn streamed_pcm_uses_the_same_sender_and_seek_offset() {
    let (_root, mut backend) = fixture(false);
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let BufferedInput::Bytes(bytes) = wav(2) else { unreachable!() };
    tx.try_send(bytes::Bytes::copy_from_slice(&bytes)).unwrap(); drop(tx);
    backend.start_stream(21, StreamingInput::new(rx, crate::audio::rodio_backend::StreamFailure::default(), None, Duration::from_secs(1))).unwrap();
    wait_started(&mut backend);
    assert!(backend.position().unwrap() >= Duration::from_secs(1));
    backend.stop();
}
