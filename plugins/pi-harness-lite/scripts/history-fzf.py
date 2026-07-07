#!/usr/bin/env python3
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

MAX_FILES = 2000
MAX_ROWS = 5000
MAX_LIST_SNIPPET_CHARS = 120
MAX_PREVIEW_CHARS = 2000
AGENT_PHASES = {None, "final_answer"}


@dataclass
class HistoryRow:
    timestamp: str
    thread_id: str
    role: str
    cwd: str
    snippet: str
    preview: str
    file_mtime: float


@dataclass
class RolloutMeta:
    timestamp: str
    thread_id: str
    cwd: str
    file_mtime: float


def codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")).expanduser()


def current_project_scope() -> Path:
    cwd = Path.cwd()
    git = shutil.which("git")
    if git is not None:
        try:
            result = subprocess.run(
                [git, "rev-parse", "--show-toplevel"],
                cwd=cwd,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                timeout=1,
                check=False,
            )
            root = result.stdout.strip()
            if result.returncode == 0 and root:
                return Path(root).expanduser()
        except (OSError, subprocess.SubprocessError):
            pass
    return cwd


def path_in_scope(path: str, scope: Path) -> bool:
    if not path:
        return False
    try:
        candidate = os.path.abspath(os.path.expanduser(path))
        scope_path = os.path.abspath(os.path.expanduser(str(scope)))
        return os.path.commonpath([candidate, scope_path]) == scope_path
    except ValueError:
        return False


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


def is_internal_text(text: str) -> bool:
    text = clean_text(text)
    return text.startswith(
        (
            "<environment_context>",
            "<codex_internal_context",
            "# AGENTS.md instructions",
        )
    )


def shorten(text: str, limit: int) -> str:
    text = clean_text(text)
    if len(text) <= limit:
        return text
    return f"{text[: limit - 1]}..."


def timestamp_label(timestamp: str) -> str:
    if len(timestamp) >= 16 and timestamp[4] == "-" and timestamp[10] == "T":
        return f"{timestamp[5:10]} {timestamp[11:16]}"
    return timestamp or "-"


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


def rollout_meta(path: Path) -> RolloutMeta:
    thread_id = thread_id_from_rollout_path(path)
    timestamp = ""
    cwd = ""
    try:
        mtime = path.stat().st_mtime
        for raw_line in rollout_lines(path):
            try:
                record = json.loads(raw_line)
            except json.JSONDecodeError:
                continue
            if record.get("type") != "session_meta":
                continue
            payload = record.get("payload")
            if not isinstance(payload, dict):
                continue
            thread_id = str(payload.get("id") or thread_id)
            timestamp = str(payload.get("timestamp") or timestamp)
            cwd = str(payload.get("cwd") or cwd)
            break
    except OSError:
        mtime = 0.0
    return RolloutMeta(
        timestamp=timestamp or "-",
        thread_id=thread_id,
        cwd=cwd,
        file_mtime=mtime,
    )


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


def append_row(
    rows: list[HistoryRow],
    seen: set[tuple[str, str]],
    meta: RolloutMeta,
    role: str,
    text: str,
    timestamp: str,
) -> None:
    if is_internal_text(text):
        return
    text = clean_text(text)
    if not text:
        return
    key = (role, text)
    if key in seen:
        return
    seen.add(key)
    rows.append(
        HistoryRow(
            timestamp=timestamp or meta.timestamp,
            thread_id=meta.thread_id,
            role=role,
            cwd=cwd_label(meta.cwd),
            snippet=shorten(text, MAX_LIST_SNIPPET_CHARS),
            preview=shorten(text, MAX_PREVIEW_CHARS),
            file_mtime=meta.file_mtime,
        )
    )


def event_rows_from_rollout(path: Path, meta: RolloutMeta) -> list[HistoryRow]:
    rows: list[HistoryRow] = []
    seen: set[tuple[str, str]] = set()
    try:
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
                meta.thread_id = str(payload.get("id") or meta.thread_id)
                meta.cwd = str(payload.get("cwd") or meta.cwd)
                meta.timestamp = str(payload.get("timestamp") or meta.timestamp)
                continue

            if record_type != "event_msg" or payload_type not in {
                "user_message",
                "agent_message",
            }:
                continue

            if payload_type == "agent_message" and payload.get("phase") not in AGENT_PHASES:
                continue
            message = payload.get("message")
            if isinstance(message, str):
                role = "user" if payload_type == "user_message" else "agent"
                append_row(rows, seen, meta, role, message, str(record.get("timestamp") or ""))
    except OSError:
        return rows
    return rows


def response_rows_from_rollout(path: Path, meta: RolloutMeta) -> list[HistoryRow]:
    rows: list[HistoryRow] = []
    seen: set[tuple[str, str]] = set()
    try:
        for raw_line in rollout_lines(path):
            try:
                record = json.loads(raw_line)
            except json.JSONDecodeError:
                continue
            payload = record.get("payload")
            if not isinstance(payload, dict):
                continue
            if record.get("type") != "response_item" or payload.get("type") != "message":
                continue
            payload_role = payload.get("role")
            if payload_role == "user":
                role = "user"
            elif payload_role == "assistant" and payload.get("phase") in AGENT_PHASES:
                role = "agent"
            else:
                continue
            text = text_from_response_content(payload.get("content"))
            if text is not None:
                append_row(rows, seen, meta, role, text, str(record.get("timestamp") or ""))
    except OSError:
        return rows
    return rows


def rows_from_rollout(path: Path, meta: RolloutMeta) -> list[HistoryRow]:
    rows = event_rows_from_rollout(path, meta)
    if rows:
        return rows
    return response_rows_from_rollout(path, meta)


def load_rows() -> list[HistoryRow]:
    rows: list[HistoryRow] = []
    scope = current_project_scope()
    scope_mode = os.environ.get("CODEX_HISTORY_FZF_SCOPE", "project").strip().lower()
    for path in iter_rollout_files(codex_home()):
        meta = rollout_meta(path)
        if scope_mode != "all" and not path_in_scope(meta.cwd, scope):
            continue
        rows.extend(rows_from_rollout(path, meta))
        if len(rows) >= MAX_ROWS:
            break
    rows.sort(key=lambda row: (row.file_mtime, row.timestamp), reverse=True)
    deduped: list[HistoryRow] = []
    seen: set[tuple[str, str, str]] = set()
    for row in rows:
        key = (row.thread_id, row.role, row.snippet)
        if key in seen:
            continue
        seen.add(key)
        deduped.append(row)
    return deduped[:MAX_ROWS]


def fzf_input(rows: list[HistoryRow]) -> str:
    return "\n".join(
        "\t".join(
            [
                timestamp_label(row.timestamp),
                row.thread_id,
                row.role,
                row.cwd,
                row.snippet,
                row.preview,
            ]
        )
        for row in rows
    )


def run_fzf(rows: list[HistoryRow]) -> str | None:
    fzf = shutil.which("fzf")
    if fzf is None:
        print("history-fzf needs fzf on PATH.", file=sys.stderr)
        return None
    preview = "printf 'time: %s\\nthread: %s\\nrole: %s\\ncwd: %s\\n\\n%s\\n' {1} {2} {3} {4} {6}"

    process = subprocess.run(
        [
            fzf,
            "--ansi",
            "--delimiter",
            "\t",
            "--with-nth",
            "1,3,4,5",
            "--accept-nth",
            "2",
            "--wrap",
            "--wrap-sign",
            "  ",
            "--highlight-line",
            "--prompt",
            "Codex project history> ",
            "--header",
            f"{len(rows)} rows from {current_project_scope()}",
            "--preview",
            preview,
            "--preview-window",
            "right,60%,wrap,border-left",
            "--preview-label",
            " message ",
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
    if len(parts) >= 2:
        return parts[1].strip() or None
    return selected


def main() -> int:
    rows = load_rows()
    if not rows:
        print(
            f"No Codex conversation history found for {current_project_scope()}.",
            file=sys.stderr,
        )
        return 0

    thread_id = run_fzf(rows)
    if thread_id:
        print(thread_id)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
