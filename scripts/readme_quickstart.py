#!/usr/bin/env python3
"""Run the README quick start with a stub GM.

Reads the Quick start section of README.md, runs that OS's fenced commands,
seats a stub model that commits the queued lines, and checks the ending line
and `spook verify`.

Set SPOOK_QUICKSTART_BIN to a directory that already holds `spook` to skip
the `cargo build` line. The checkout this script runs in is the fresh clone.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FENCE = re.compile(r"```[^\n]*\n(.*?)```", re.S)
STUB_BODY = json.dumps(
    {
        "id": "stub",
        "object": "chat.completion",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "act",
                                "arguments": json.dumps({"says": [], "intents": []}),
                            },
                        }
                    ],
                },
            }
        ],
    }
).encode()


class StubServer(ThreadingHTTPServer):
    def handle_error(self, request, client_address) -> None:
        err = sys.exc_info()[1]
        if isinstance(err, (ConnectionResetError, BrokenPipeError, ConnectionAbortedError)):
            return
        super().handle_error(request, client_address)


class StubHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(STUB_BODY)))
        self.end_headers()
        self.wfile.write(STUB_BODY)

    def log_message(self, fmt: str, *args) -> None:
        return


class Pipe:
    def __init__(self, proc: subprocess.Popen[str]) -> None:
        self.proc = proc
        self.out: list[str] = []
        self.err: list[str] = []
        threading.Thread(target=self._drain, args=(proc.stdout, self.out), daemon=True).start()
        threading.Thread(target=self._drain, args=(proc.stderr, self.err), daemon=True).start()

    @staticmethod
    def _drain(pipe, buf: list[str]) -> None:
        if pipe is None:
            return
        for line in pipe:
            buf.append(line)


def main() -> None:
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    quick = extract_section(readme, "##", "Quick start")
    os_heading = "Windows" if sys.platform == "win32" else "Linux and macOS"
    os_body = extract_section(quick, "###", os_heading)
    lines = extract_fence(extract_section(quick, "###", "Lines")).splitlines()
    lines = [line.strip() for line in lines if line.strip()]
    ending = extract_fence(extract_section(quick, "###", "Ending")).strip()
    setup, run_line, play_line, verify_line = classify(os_body)
    if not lines or not ending or not run_line or not play_line or not verify_line:
        raise SystemExit("the Quick start section is missing a fence")
    # cargo test brings its own binary. It must not use the README folder or
    # the default ports, and it may delete only the scratch folder it made.
    scratch_root: Path | None = None
    scratch: Path | None = None
    stub: ThreadingHTTPServer | None = None
    host: Pipe | None = None
    gm: Pipe | None = None
    play: Pipe | None = None
    try:
        if os.environ.get("SPOOK_QUICKSTART_BIN"):
            scratch_root = Path(tempfile.mkdtemp(prefix="spook-readme-"))
            scratch = scratch_root / "pack"
            shutil.copytree(ROOT / "packs" / "sample-d20", scratch)
        run_setup(setup, skip_copy=scratch is not None)
        apply_bin()
        stub, stub_port = start_stub()
        if scratch is not None:
            host = spawn(["spook", "run", str(scratch), "--addr", "127.0.0.1:0"])
        else:
            host = spawn(parse_spook_args(run_line))
        game, door = wait_listening(host)
        pack = str(scratch) if scratch is not None else parse_spook_args(run_line)[2]
        env = os.environ.copy()
        env["SPOOK_GM_KEY"] = "stub"
        env["SPOOK_GM_URL"] = f"http://127.0.0.1:{stub_port}/v1"
        env["SPOOK_GM_MODEL"] = "stub"
        gm = spawn(
            ["spook", "ai-gm", pack, "--addr", game, "--mcp-http", door],
            env=env,
        )
        wait_text(gm.err, "listening", 30, "the stub GM did not sit", gm, host)
        if scratch is not None:
            play = spawn(["spook", "play", pack, "--addr", game])
        else:
            play = spawn(parse_spook_args(play_line))
        wait_text(play.out, "You are in", 30, "play did not show a place", play, host, gm)
        assert play.proc.stdin is not None
        for line in lines:
            before = "".join(play.out).count("You are in")
            play.proc.stdin.write(line + "\n")
            play.proc.stdin.flush()
            deadline = time.time() + 45
            while time.time() < deadline:
                if "".join(play.out).count("You are in") > before:
                    break
                time.sleep(0.05)
            else:
                dump(f"the line did not finish: {line}", None, play, host, gm)
        wait_text(play.out, ending, 45, "the session did not reach the ending", play, host, gm)
        assert host.proc.stdin is not None
        host.proc.stdin.write("stop\n")
        host.proc.stdin.flush()
        code = host.proc.wait(timeout=20)
        if code != 0:
            dump("the host exited", code, host, gm, play)
        print("--- play ---")
        print("".join(play.out), end="")
        verify_args = (
            ["spook", "verify", pack] if scratch is not None else parse_spook_args(verify_line)
        )
        verified = subprocess.run(
            verify_args,
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        print("--- verify ---")
        print(verified.stdout, end="")
        if verified.returncode != 0 or "verified" not in verified.stdout:
            print(verified.stderr, file=sys.stderr)
            raise SystemExit("spook verify failed")
        if not re.search(r"verified [1-9]", verified.stdout):
            raise SystemExit("spook verify applied no records")
    finally:
        stop(play)
        stop(gm)
        stop(host)
        if stub is not None:
            stub.shutdown()
        if scratch_root is not None:
            shutil.rmtree(scratch_root, ignore_errors=True)


def extract_section(text: str, level: str, title: str) -> str:
    lines = text.splitlines(keepends=True)
    heading = f"{level} {title}"
    start = None
    for index, line in enumerate(lines):
        if line.rstrip("\r\n") == heading:
            start = index + 1
            break
    if start is None:
        raise SystemExit(f"missing heading {heading}")
    end = len(lines)
    for index in range(start, len(lines)):
        if lines[index].startswith(f"{level} "):
            end = index
            break
    return "".join(lines[start:end])


def extract_fence(text: str) -> str:
    found = FENCE.findall(text)
    if not found:
        raise SystemExit("missing fence")
    return found[0].strip("\n")


def classify(text: str) -> tuple[str, str, str, str]:
    setup: list[str] = []
    run = play = verify = ""
    for body in FENCE.findall(text):
        first = body.strip().splitlines()[0].strip()
        if first.startswith("spook run"):
            run = first
        elif first.startswith("spook play"):
            play = first
        elif first.startswith("spook verify"):
            verify = first
        else:
            setup.append(body.strip("\n"))
    return "\n".join(setup), run, play, verify


def expand(text: str) -> str:
    temp = os.environ.get("TEMP", "")
    return text.replace("$env:TEMP", temp).replace("%TEMP%", temp)


def run_setup(setup: str, skip_copy: bool) -> None:
    kept: list[str] = []
    in_repo = (ROOT / "Cargo.toml").is_file()
    skip_build = bool(os.environ.get("SPOOK_QUICKSTART_BIN"))
    for line in setup.splitlines():
        stripped = line.strip()
        if in_repo and (stripped.startswith("git clone") or stripped.startswith("cd ")):
            continue
        if skip_build and stripped.startswith("cargo "):
            continue
        if skip_copy and (stripped.startswith("cp ") or stripped.lower().startswith("copy-item")):
            continue
        kept.append(line)
    body = "\n".join(kept) + "\n"
    env_file = Path(tempfile.mkdtemp(prefix="spook-qs-")) / "path"
    if sys.platform == "win32":
        quoted = "'" + str(env_file).replace("'", "''") + "'"
        script = env_file.with_suffix(".ps1")
        script.write_text(
            body + f"\n$env:PATH | Out-File -FilePath {quoted} -Encoding ascii -NoNewline\n",
            encoding="utf-8",
        )
        subprocess.run(
            ["powershell", "-NoProfile", "-File", str(script)],
            cwd=ROOT,
            check=True,
        )
    else:
        quoted = "'" + str(env_file).replace("'", "'\\''") + "'"
        subprocess.run(
            ["bash", "-c", body + f'\nprintf %s "$PATH" > {quoted}\n'],
            cwd=ROOT,
            check=True,
        )
    os.environ["PATH"] = env_file.read_text(encoding="utf-8").strip()


def apply_bin() -> None:
    bin_dir = os.environ.get("SPOOK_QUICKSTART_BIN")
    if bin_dir:
        os.environ["PATH"] = bin_dir + os.pathsep + os.environ.get("PATH", "")


def start_stub() -> tuple[ThreadingHTTPServer, int]:
    server = StubServer(("127.0.0.1", 0), StubHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, server.server_address[1]


def spawn(args: list[str], env: dict[str, str] | None = None) -> Pipe:
    proc = subprocess.Popen(
        args,
        cwd=ROOT,
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return Pipe(proc)


def parse_spook_args(line: str) -> list[str]:
    match = re.match(r"spook\s+(\S+)\s+(.+)$", expand(line.strip()))
    if not match:
        raise SystemExit(f"not a spook command: {line}")
    return ["spook", match.group(1), match.group(2).strip()]


def wait_listening(host: Pipe) -> tuple[str, str]:
    deadline = time.time() + 30
    while time.time() < deadline:
        for line in host.out:
            if '"kind"' in line and "listening" in line:
                data = json.loads(line)
                return data["game"], data["door"]
        if host.proc.poll() is not None:
            dump("the host exited before listening", host.proc.returncode, host)
        time.sleep(0.05)
    dump("the host did not listen", None, host)


def wait_text(buf: list[str], needle: str, seconds: float, message: str, *pipes: Pipe) -> None:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if any(needle in line for line in buf):
            return
        time.sleep(0.05)
    dump(message, None, *pipes)


def dump(message: str, code: int | None, *pipes: Pipe) -> None:
    print(message if code is None else f"{message} ({code})", file=sys.stderr)
    for index, pipe in enumerate(pipes):
        print(f"--- {index} stdout ---", file=sys.stderr)
        print("".join(pipe.out)[-4000:], file=sys.stderr)
        print(f"--- {index} stderr ---", file=sys.stderr)
        print("".join(pipe.err)[-4000:], file=sys.stderr)
    raise SystemExit(1)


def stop(pipe: Pipe | None) -> None:
    if pipe is None or pipe.proc.poll() is not None:
        return
    pipe.proc.kill()
    pipe.proc.wait(timeout=10)


if __name__ == "__main__":
    main()
