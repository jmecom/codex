#!/usr/bin/env python3
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

MAX_FILES = 5000
MAX_ROWS = 30000
MAX_SNIPPET_CHARS = 260


@dataclass
class HistoryRow:
    timestamp: str
    thread_id: str
    role: str
    cwd: str
    snippet: str
    file_mtime: float


def codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")).expanduser()


def iter_rollout_files(home: Path) -> list[Path]:
    roots = [home / "sessions", home / "archived_sessions"]
    files: list[Path] = []
    for root in roots:
        if not root.exists():
            continue
        files.extend(root.rglob("rollout-*.jsonl"))
        files.extend(root.rglob("rollout-*.jsonl.zst"))
    files.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    return files[:MAX_FILES]


def clean_text(text: str) -> str:
    return " ".join(text.replace("\t", " ").split())


def shorten(text: str) -> str:
    text = clean_text(text)
    if len(text) <= MAX_SNIPPET_CHARS:
        return text
    return f"{text[: MAX_SNIPPET_CHARS - 1]}..."


def cwd_label(cwd: str) -> str:
    if not cwd:
        return "-"
    path = Path(cwd).expanduser()
    if len(path.parts) >= 2:
        return "/".join(path.parts[-2:])
    return str(path)


def text_from_response_content(content: object) -> str | None:
    if not isinstance(content, list):
        return None
    parts: list[str] = []
    for item in content:
        if not isinstance(item, dict):
            continue
        text_type = item.get("type")
        if text_type not in {"input_text", "output_text"}:
            continue
        text = item.get("text")
        if isinstance(text, str):
            parts.append(text)
    text = clean_text(" ".join(parts))
    if text.startswith("<environment_context>"):
        return None
    return text or None


def thread_id_from_rollout_path(path: Path) -> str:
    name = path.name
    for suffix in [".jsonl.zst", ".jsonl"]:
        if name.endswith(suffix):
            name = name[: -len(suffix)]
            break
    return name.removeprefix("rollout-")


def rollout_lines(path: Path):
    if path.name.endswith(".jsonl.zst"):
        zstd = shutil.which("zstd")
        if zstd is None:
            return
        process = subprocess.Popen(
            [zstd, "-dc", str(path)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        if process.stdout is None:
            return
        with process.stdout:
            for line in process.stdout:
                yield line
        process.wait()
        return

    with path.open("r", encoding="utf-8") as handle:
        yield from handle


def rows_from_rollout(path: Path) -> list[HistoryRow]:
    rows: list[HistoryRow] = []
    thread_id = thread_id_from_rollout_path(path)
    cwd = ""
    timestamp = ""
    try:
        mtime = path.stat().st_mtime
        for raw_line in rollout_lines(path):
            try:
                record = json.loads(raw_line)
            except json.JSONDecodeError:
                continue
            payload = record.get("payload")
            if not isinstance(payload, dict):
                continue

            record_type = record.get("type")
            payload_type = payload.get("type")
            if record_type == "session_meta":
                thread_id = str(payload.get("id") or thread_id)
                cwd = str(payload.get("cwd") or cwd)
                timestamp = str(payload.get("timestamp") or timestamp)
                continue

            role: str | None = None
            text: str | None = None
            if record_type == "event_msg" and payload_type in {
                "user_message",
                "agent_message",
            }:
                role = "user" if payload_type == "user_message" else "agent"
                message = payload.get("message")
                if isinstance(message, str):
                    text = message
            elif record_type == "response_item" and payload_type == "message":
                payload_role = payload.get("role")
                if payload_role == "user":
                    role = "user"
                elif payload_role == "assistant":
                    role = "agent"
                text = text_from_response_content(payload.get("content"))

            if role is None or text is None:
                continue
            snippet = shorten(text)
            if snippet:
                rows.append(
                    HistoryRow(
                        timestamp=timestamp or "-",
                        thread_id=thread_id,
                        role=role,
                        cwd=cwd_label(cwd),
                        snippet=snippet,
                        file_mtime=mtime,
                    )
                )
    except OSError:
        return rows
    return rows


def load_rows() -> list[HistoryRow]:
    rows: list[HistoryRow] = []
    for path in iter_rollout_files(codex_home()):
        rows.extend(rows_from_rollout(path))
        if len(rows) >= MAX_ROWS:
            break
    rows.sort(key=lambda row: row.file_mtime, reverse=True)
    return rows[:MAX_ROWS]


def fzf_input(rows: list[HistoryRow]) -> str:
    return "\n".join(
        "\t".join([row.timestamp, row.thread_id, row.role, row.cwd, row.snippet])
        for row in rows
    )


def run_fzf(rows: list[HistoryRow]) -> str | None:
    fzf = shutil.which("fzf")
    if fzf is None:
        print("history-fzf needs fzf on PATH.", file=sys.stderr)
        return None

    process = subprocess.run(
        [
            fzf,
            "--ansi",
            "--delimiter",
            "\t",
            "--with-nth",
            "1,3,4,5",
            "--prompt",
            "Codex history> ",
            "--height",
            "80%",
            "--layout",
            "reverse",
            "--border",
            "rounded",
        ],
        input=fzf_input(rows),
        text=True,
        stdout=subprocess.PIPE,
        stderr=None,
        check=False,
    )
    if process.returncode != 0:
        return None

    selected = process.stdout.strip()
    if not selected:
        return None
    parts = selected.split("\t")
    if len(parts) < 2:
        return None
    return parts[1].strip() or None


def main() -> int:
    rows = load_rows()
    if not rows:
        print("No Codex conversation history found.", file=sys.stderr)
        return 0

    thread_id = run_fzf(rows)
    if thread_id:
        print(thread_id)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
