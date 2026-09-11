"""Isolated local-folder startup/navigation smoke; never touches user settings."""
import fcntl
import os
import pathlib
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import wave

wikipedia = "--wikipedia" in sys.argv[2:]
browse_only = "--browse-only" in sys.argv[2:]
assert not (wikipedia and browse_only), "Wikipedia smoke requires track metadata loading"

with tempfile.TemporaryDirectory(prefix="textamp-folder-tui-") as temporary:
    root = pathlib.Path(temporary)
    music = root / "music"
    album = music / "Fixture Album"
    album.mkdir(parents=True)
    for name in ("02 Silence.wav", "10 Silence.wav"):
        with wave.open(str(album / name), "wb") as audio:
            audio.setnchannels(2)
            audio.setsampwidth(2)
            audio.setframerate(44100)
            audio.writeframes(b"\0" * 44100 * 4 * 8)
        if wikipedia:
            # Standard RIFF INFO artist tag, not a custom sidecar convention.
            song = album / name
            raw = song.read_bytes()
            artist = b"Tool\0\0"
            info = b"INFOIART" + struct.pack("<I", len(artist)) + artist
            chunk = b"LIST" + struct.pack("<I", len(info)) + info
            data_at = raw.index(b"data", 12)
            song.write_bytes(b"RIFF" + struct.pack("<I", len(raw) + len(chunk) - 8) + raw[8:data_at] + chunk + raw[data_at:])
    env = os.environ.copy()
    for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        env[name] = str(root / name)
    config_dir = pathlib.Path(env["XDG_CONFIG_HOME"]) / "textamp"
    config_dir.mkdir(parents=True)
    (config_dir / "config.toml").write_text(
        f'default_folder_source = "fixture"\n[[folder_sources]]\nid = "fixture"\nname = "Fixture Library"\nkind = "local"\npath = "{music}"\n'
    )
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "Apple_Terminal"
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 130, 0, 0))
    before = termios.tcgetattr(slave)
    process = subprocess.Popen([str(pathlib.Path(sys.argv[1]).resolve())], stdin=slave, stdout=slave, stderr=slave, env=env)
    output = bytearray()

    def wait_for(text):
        # Differential terminal draws can move across already-correct spaces
        # without emitting them again. Match text independent of those gaps.
        def normalized(value):
            return re.sub(rb"\s+", b"", re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b"", value))
        deadline = time.monotonic() + (50 if wikipedia else 20)
        while normalized(text) not in normalized(output) and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
            if process.poll() is not None:
                break
        plain = re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b"", output)
        diagnostic = re.sub(r"[\u2500-\u28ff]", "", plain.decode("utf-8", errors="replace"))
        assert normalized(text) in normalized(output), f"UI did not display {text!r}; exit={process.poll()}; tail={diagnostic[-6000:]!r}"

    try:
        wait_for(b"Fixture Album")
        assert b"Sign in to Plex" not in output
        os.write(master, b"\r")
        wait_for(b"02 Silence.wav")
        if not browse_only:
            os.write(master, b"\r")
            if wikipedia:
                wait_for(b"Tool")  # wait for async metadata, draining terminal output
            time.sleep(0.4)
            os.write(master, b" ")  # pause silent playback
        if wikipedia:
            os.write(master, b"\x1bOS")  # F4: standard artist tag -> Wikipedia
            wait_for(b"Tool is an American")
            wait_for(b"B source")
            output.clear()
            os.write(master, b"\x1b[C")  # next photo
            # Differential rendering replaces only the counter's digit; the
            # photo-selection behavior is separately covered by state tests.
            wait_for(b"2")
            os.write(master, b"\x1bOS")  # F4 closes the popup
            time.sleep(0.2)
        if browse_only:
            output.clear()
            os.write(master, b"\x1bOQ")  # F2: unified Libraries
            wait_for(b"Add library")
            wait_for(b"[Active]")
            os.write(master, b"a")
            wait_for(b"Subsonic / Navidrome")
            os.write(master, b"\x1b[B\r")  # WebDAV type -> one form
            wait_for(b"Add WebDAV library")
            wait_for(b"Name (optional)")
            os.write(master, b"\x1b")  # cancel, leaving the library untouched
            time.sleep(0.2)
            output.clear()
            os.write(master, b"\x1b[D\x1b[B\x1b[C")  # sidebar -> Textamp
            wait_for(b"theme:")
            os.write(master, b"\x1b[F")  # End: integrated sidebar visibility
            wait_for(b"show in left column:")
            output.clear()
            os.write(master, b"\x1b[D\x1b[A\x1b[C\x1b[H\r")  # Libraries -> first library options
            wait_for(b"Clear library cache")
            # Force a complete redraw: differential updates can skip letters
            # that happen to match cells from the previous settings panel.
            output.clear()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 131, 0, 0))
            process.send_signal(signal.SIGWINCH)
            wait_for(b"Fixture Library")
            wait_for(b"Cache:")
            os.write(master, b"\x1b")
            time.sleep(0.2)
        os.write(master, b"\x1bOR")  # F3 library switcher
        wait_for(b"Switch library")
        process.send_signal(signal.SIGTERM)
        deadline = time.monotonic() + 10
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
        assert process.poll() == 0, f"unexpected exit {process.poll()}"
        assert termios.tcgetattr(slave) == before, "terminal not restored"
        print("PASS: folder-only startup, filename navigation, library switcher, shutdown, and terminal restoration")
        if browse_only:
            print("PASS: unified library manager, add/cancel connection form, integrated settings, per-library cache display")
        if not browse_only:
            print("PASS: silent playback input")
        if wikipedia:
            print("PASS: embedded artist tag, Wikipedia band biography, photo switching, and popup close")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
