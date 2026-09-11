"""PTY smoke against the dedicated localhost:14533 Navidrome fixture.

The fixture has two generated silent FLACs, a Fixture Artist/Fixture Album,
and account fixture / fixture-password. Never uses the user's configuration.
"""
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

with tempfile.TemporaryDirectory(prefix="textamp-navidrome-tui-") as temporary:
    root = pathlib.Path(temporary)
    env = os.environ.copy()
    for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        env[name] = str(root / name)
        (pathlib.Path(env[name]) / "textamp").mkdir(parents=True)
    config = pathlib.Path(env["XDG_CONFIG_HOME"]) / "textamp/config.toml"
    config.write_text('''[default_navidrome]
source_id = "fixture"
[[navidrome_sources]]
id = "fixture"
name = "Fixture account"
url = "http://127.0.0.1:14533"
username = "fixture"
''')
    secret = pathlib.Path(env["XDG_DATA_HOME"]) / "textamp/navidrome-credentials.toml"
    secret.write_text('fixture = "fixture-password"\n')
    secret.chmod(0o600)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "Apple_Terminal"
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
    before = termios.tcgetattr(slave)
    process = subprocess.Popen([str(pathlib.Path(sys.argv[1]).resolve())], stdin=slave, stdout=slave, stderr=slave, env=env)
    output = bytearray()

    def wait_for(text):
        def normalized(value):
            return re.sub(rb"\s+", b"", re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b"", value))
        deadline = time.monotonic() + 20
        while normalized(text) not in normalized(output) and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
            if process.poll() is not None:
                break
        plain = re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b"", output)
        diagnostic = re.sub(r"[\u2500-\u28ff]", "", plain.decode("utf-8", errors="replace"))
        assert normalized(text) in normalized(output), f"Missing {text!r}; exit={process.poll()}; output={diagnostic[-6000:]!r}"

    try:
        wait_for(b"Fixture Artist")
        assert b"Sign in to Plex" not in output
        os.write(master, b"\r")
        wait_for(b"Fixture Album")
        os.write(master, b"\r")
        wait_for(b"First Song")
        wait_for(b"Second Song")
        os.write(master, b"\r")
        wait_for("⏸".encode())  # transport has entered playback; differential draws omit unchanged digits
        os.write(master, b"\x0e")  # Ctrl+N: waveform/spectrum view
        time.sleep(0.5)
        os.write(master, b" ")  # pause
        os.write(master, b"\x1bOR")  # F3: switcher
        wait_for(b"Fixture account")
        wait_for(b"Music Library")
        wait_for(b"Switch library")
        os.write(master, b"\x1b")
        time.sleep(0.2)
        os.write(master, b":lyrics")
        wait_for(b"Lyrics")
        os.write(master, b"\x1b")
        time.sleep(0.2)
        os.write(master, b"\x1bOR")
        time.sleep(0.2)
        os.write(master, b"\x1bOQ")  # F2: Settings → Libraries
        wait_for(b"Add library")
        os.write(master, b"c")  # edit the existing connection, not a duplicate account
        wait_for(b"Subsonic / Navidrome connection")
        wait_for(b"Name (optional)")
        output.clear()
        os.write(master, b"\t\tfixture-password\t\r")
        wait_for(b"Fixture Artist")
        assert b"fixture-password" not in output, "password was displayed"
        os.write(master, b"\x1bOR")
        wait_for(b"Fixture account")
        process.send_signal(signal.SIGTERM)
        deadline = time.monotonic() + 10
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
        assert process.poll() == 0
        assert termios.tcgetattr(slave) == before, "terminal not restored"
        saved = config.read_text()
        assert "fixture-password" not in saved
        assert "[[navidrome_sources.libraries]]" in saved, "discovered libraries were not persisted"
        assert saved.count("[[navidrome_sources]]") == 1, "editing created a duplicate account"
        assert secret.stat().st_mode & 0o777 == 0o600
        log = pathlib.Path(env["XDG_STATE_HOME"]) / "textamp/textamp.log"
        if log.exists():
            assert "fixture-password" not in log.read_text()
        print("PASS: Navidrome startup, artist/album/track navigation, playback controls, visualizer view, library manager, command palette, masked connection edit, private persistence, terminal restoration")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
