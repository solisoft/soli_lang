#!/usr/bin/env python3
"""Agents on Soli — copy fixture, run a frozen CLI, grade with soli.

Runners (first board):
  claude    — `claude -p` (Claude Code)
  opencode  — `opencode run` with DeepSeek (opencode/deepseek-v4-flash-free)
  grok      — `grok -p` / `--single` (Grok Build)

  python3 scripts/evals/run.py --list
  python3 scripts/evals/run.py --grade-fixture
  python3 scripts/evals/run.py --dry-run --models claude,opencode,grok --tasks hello-world --runs 1
  python3 scripts/evals/run.py --models claude,opencode,grok --tasks hello-world --runs 1
  python3 scripts/evals/run.py --models claude,opencode,grok --out www/data/ai_evals.json

Does not invent scores. Missing token/cost from a CLI is stored as null.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from datetime import date, datetime, timezone
from pathlib import Path
from shutil import which

ROOT = Path(__file__).resolve().parents[2]
APP = ROOT / "evals" / "app"
TASKS = ROOT / "evals" / "tasks"
HARNESS = "soli-evals/0.2"
DEFAULT_RUNS = 3
DEFAULT_TIMEOUT = 600

RUNNERS = {
    "claude": {
        "id": "claude-code",
        "label": "Claude Code",
        "bin": "claude",
        "model": os.environ.get("SOLI_EVALS_CLAUDE_MODEL", "sonnet"),
    },
    "opencode": {
        "id": "opencode-deepseek",
        "label": "OpenCode (DeepSeek)",
        "bin": "opencode",
        "model": os.environ.get(
            "SOLI_EVALS_OPENCODE_MODEL", "opencode/deepseek-v4-flash-free"
        ),
    },
    "grok": {
        "id": "grok-build",
        "label": "Grok Build",
        "bin": "grok",
        "model": os.environ.get("SOLI_EVALS_GROK_MODEL", "grok-4.6"),
    },
}


def task_slugs() -> list[str]:
    return sorted(p.name for p in TASKS.iterdir() if p.is_dir())


def run(cmd: list[str], cwd: Path, timeout: int | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        text=True,
        capture_output=True,
        timeout=timeout,
    )


def copy_fixture(dest: Path) -> None:
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(APP, dest, ignore=shutil.ignore_patterns(".git"))


def copy_hidden(task: str, dest: Path) -> None:
    hidden = TASKS / task / "hidden"
    target = dest / "hidden"
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(hidden, target)


def wrap_prompt(task: str) -> str:
    body = (TASKS / task / "prompt.md").read_text()
    return f"""You are editing a small Soli MVC app in the current working directory.
Soli files use the .sl extension. Views use .html.slv.
Implement ONLY the task below. Do not add unrelated files or a README.
Do not invent SDBQL. Prefer hash .where({{...}}) when querying.
Do not run a long-lived server. You may run `soli lint` on files you edit.
When the task is done, stop.

# Task ({task})

{body}
"""


def api_hits(task: str, dest: Path) -> tuple[int, int]:
    expect = (TASKS / task / "expect.md").read_text()
    needles = [
        line[2:].strip().strip("`")
        for line in expect.splitlines()
        if line.startswith("- `") and "`" in line[2:]
    ]
    blob = ""
    for path in dest.rglob("*"):
        if not path.is_file() or path.suffix not in {".sl", ".slv", ".md"}:
            continue
        if "hidden" in path.parts:
            continue
        if path.name.startswith(".soli-eval"):
            continue
        try:
            blob += path.read_text(errors="ignore")
        except OSError:
            pass
    if not needles:
        return 0, 0
    hits = sum(1 for n in needles if n in blob)
    return hits, len(needles)


def grade_task(task: str, dest: Path) -> dict:
    copy_hidden(task, dest)
    lint = run(["soli", "lint"], dest)
    test = run(["soli", "test", "hidden", "--no-coverage"], dest)
    passed = lint.returncode == 0 and test.returncode == 0
    hits, total = api_hits(task, dest)
    return {
        "task": task,
        "passed": passed,
        "lint_ok": lint.returncode == 0,
        "test_ok": test.returncode == 0,
        "api_hits": hits,
        "api_total": total,
        "lint_stderr": (lint.stderr or "")[-2000:],
        "test_stderr": (test.stderr or "")[-2000:],
    }


def grade_fixture() -> int:
    dest = Path("/tmp/soli-evals-fixture")
    copy_fixture(dest)
    lint = run(["soli", "lint"], dest)
    test = run(["soli", "test", "tests", "--no-coverage"], dest)
    print(lint.stdout)
    print(lint.stderr)
    print(test.stdout)
    print(test.stderr)
    if lint.returncode != 0 or test.returncode != 0:
        print("fixture grade: FAIL", file=sys.stderr)
        return 1
    print("fixture grade: PASS")
    return 0


def empty_board() -> dict:
    return {
        "generated_at": None,
        "harness": HARNESS,
        "runs_per_model": DEFAULT_RUNS,
        "task_count": len(task_slugs()),
        "tasks": task_slugs(),
        "runners": {k: {"id": v["id"], "model": v["model"]} for k, v in RUNNERS.items()},
        "models": [],
        "evaluations": [],
        "note": "No paid run committed yet. Do not invent scores.",
    }


def _walk_nums(obj, keys: set[str]) -> list[float]:
    found: list[float] = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in keys and isinstance(v, (int, float)):
                found.append(float(v))
            found.extend(_walk_nums(v, keys))
    elif isinstance(obj, list):
        for item in obj:
            found.extend(_walk_nums(item, keys))
    return found


def _first_str(obj, keys: set[str]) -> str | None:
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in keys and isinstance(v, str) and v:
                return v
            got = _first_str(v, keys)
            if got:
                return got
    elif isinstance(obj, list):
        for item in obj:
            got = _first_str(item, keys)
            if got:
                return got
    return None


def parse_json_blobs(text: str) -> list[object]:
    blobs: list[object] = []
    text = text.strip()
    if not text:
        return blobs
    try:
        blobs.append(json.loads(text))
        return blobs
    except json.JSONDecodeError:
        pass
    for line in text.splitlines():
        line = line.strip()
        if not line or line[0] not in "{[":
            continue
        try:
            blobs.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return blobs


def extract_usage(stdout: str, stderr: str) -> dict:
    tokens = None
    cost = None
    model = None
    for blob in parse_json_blobs(stdout) + parse_json_blobs(stderr):
        if model is None:
            model = _first_str(blob, {"model", "model_id", "modelID"})
        ins = _walk_nums(blob, {"input_tokens", "inputTokens", "prompt_tokens"})
        outs = _walk_nums(
            blob, {"output_tokens", "outputTokens", "completion_tokens"}
        )
        totals = _walk_nums(blob, {"total_tokens", "totalTokens", "tokens"})
        costs = _walk_nums(blob, {"total_cost_usd", "cost_usd", "cost", "total_cost"})
        if ins or outs:
            tokens = (sum(ins) if ins else 0) + (sum(outs) if outs else 0)
        elif totals:
            tokens = max(totals)
        if costs:
            # CLI-reported USD only; ignore huge "cost" that looks like tokens
            usd = [c for c in costs if 0 <= c < 100]
            if usd:
                cost = max(usd)
    return {"tokens": tokens, "cost_usd": cost, "reported_model": model}


def runner_cmd(name: str, dest: Path, prompt_path: Path, prompt: str) -> list[str]:
    spec = RUNNERS[name]
    model = spec["model"]
    if name == "claude":
        return [
            spec["bin"],
            "-p",
            prompt,
            "--output-format",
            "json",
            "--dangerously-skip-permissions",
            "--model",
            model,
        ]
    if name == "opencode":
        return [
            spec["bin"],
            "run",
            "--dir",
            str(dest),
            "--format",
            "json",
            "--dangerously-skip-permissions",
            "-m",
            model,
            prompt,
        ]
    if name == "grok":
        return [
            spec["bin"],
            "--cwd",
            str(dest),
            "--always-approve",
            "--output-format",
            "json",
            "--max-turns",
            "25",
            "--model",
            model,
            "--prompt-file",
            str(prompt_path),
        ]
    raise KeyError(name)


def invoke_runner(name: str, dest: Path, prompt: str, timeout: int) -> dict:
    spec = RUNNERS[name]
    bin_path = which(spec["bin"])
    if not bin_path:
        return {
            "ok": False,
            "error": f"{spec['bin']} not on PATH",
            "tokens": None,
            "cost_usd": None,
            "reported_model": None,
            "exit": None,
            "stdout_tail": "",
            "stderr_tail": "",
        }
    prompt_path = dest / ".soli-eval-prompt.md"
    prompt_path.write_text(prompt)
    cmd = runner_cmd(name, dest, prompt_path, prompt)
    try:
        proc = run(cmd, dest, timeout=timeout)
        usage = extract_usage(proc.stdout or "", proc.stderr or "")
        return {
            "ok": proc.returncode == 0,
            "error": None if proc.returncode == 0 else f"exit {proc.returncode}",
            "exit": proc.returncode,
            "tokens": usage["tokens"],
            "cost_usd": usage["cost_usd"],
            "reported_model": usage["reported_model"],
            "stdout_tail": (proc.stdout or "")[-3000:],
            "stderr_tail": (proc.stderr or "")[-3000:],
        }
    except subprocess.TimeoutExpired:
        return {
            "ok": False,
            "error": f"timeout after {timeout}s",
            "tokens": None,
            "cost_usd": None,
            "reported_model": None,
            "exit": None,
            "stdout_tail": "",
            "stderr_tail": "",
        }
    finally:
        if prompt_path.exists():
            prompt_path.unlink()


def median(values: list[float]) -> float | None:
    if not values:
        return None
    return float(statistics.median(values))


def mean(values: list[float]) -> float | None:
    if not values:
        return None
    return float(statistics.mean(values))


def aggregate(model_names: list[str], evaluations: list[dict]) -> list[dict]:
    models_out = []
    for name in model_names:
        rows = [e for e in evaluations if e["runner"] == name]
        if not rows:
            continue
        spec = RUNNERS[name]
        n = len(rows)
        passes = sum(1 for e in rows if e["passed"])
        recalls = [
            e["api_hits"] / e["api_total"]
            for e in rows
            if e["api_total"]
        ]
        token_vals = [e["tokens"] for e in rows if e["tokens"] is not None]
        cost_vals = [e["cost_usd"] for e in rows if e["cost_usd"] is not None]
        speed_vals = [e["speed_s"] for e in rows]
        models_out.append(
            {
                "id": spec["id"],
                "label": spec["label"],
                "runner": name,
                "model": spec["model"],
                "accuracy": round(passes / n, 4) if n else None,
                "speed_s": round(median(speed_vals) or 0, 1),
                "tokens": round(mean(token_vals), 0) if token_vals else None,
                "cost_usd": round(mean(cost_vals), 4) if cost_vals else None,
                "api_recall": round(mean(recalls), 4) if recalls else None,
                "n": n,
                "passes": passes,
            }
        )
    return models_out


def write_board(
    out: Path,
    model_names: list[str],
    tasks: list[str],
    runs: int,
    evaluations: list[dict],
    note: str,
) -> None:
    board = {
        "generated_at": datetime.now(timezone.utc).date().isoformat(),
        "harness": HARNESS,
        "runs_per_model": runs,
        "task_count": len(tasks),
        "tasks": tasks,
        "runners": {
            k: {"id": RUNNERS[k]["id"], "model": RUNNERS[k]["model"]}
            for k in model_names
        },
        "models": aggregate(model_names, evaluations),
        "evaluations": evaluations,
        "note": note,
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(out.suffix + ".tmp")
    tmp.write_text(json.dumps(board, indent=2) + "\n")
    tmp.replace(out)


def run_board(
    model_names: list[str],
    tasks: list[str],
    runs: int,
    out: Path,
    dry_run: bool,
    timeout: int,
    keep: bool,
) -> int:
    unknown = [m for m in model_names if m not in RUNNERS]
    if unknown:
        print(f"unknown runners: {unknown}. Known: {list(RUNNERS)}", file=sys.stderr)
        return 2
    missing_tasks = [t for t in tasks if t not in task_slugs()]
    if missing_tasks:
        print(f"unknown tasks: {missing_tasks}", file=sys.stderr)
        return 2

    evaluations: list[dict] = []
    print(
        f"harness {HARNESS}: {len(model_names)} runners × {len(tasks)} tasks × {runs} runs",
        file=sys.stderr,
    )

    for name in model_names:
        spec = RUNNERS[name]
        if dry_run:
            dest = Path("/tmp/soli-evals/_dry")
            prompt = wrap_prompt(tasks[0])
            prompt_path = dest / ".soli-eval-prompt.md"
            print(f"# {name} ({spec['id']}, model={spec['model']})")
            print(" ".join(runner_cmd(name, dest, prompt_path, prompt)[:8]), "...")
            continue
        if not which(spec["bin"]):
            print(f"skip {name}: {spec['bin']} not on PATH", file=sys.stderr)
            continue
        for task in tasks:
            for n in range(1, runs + 1):
                dest = Path(f"/tmp/soli-evals/{name}/{task}/{n}")
                print(f"→ {name} {task} run {n}/{runs}  ({dest})", file=sys.stderr)
                copy_fixture(dest)
                prompt = wrap_prompt(task)
                started = time.monotonic()
                agent = invoke_runner(name, dest, prompt, timeout)
                elapsed = time.monotonic() - started
                grade = grade_task(task, dest)
                row = {
                    "runner": name,
                    "id": spec["id"],
                    "model": spec["model"],
                    "reported_model": agent["reported_model"],
                    "task": task,
                    "run": n,
                    "passed": grade["passed"] and agent["error"] != f"timeout after {timeout}s",
                    "lint_ok": grade["lint_ok"],
                    "test_ok": grade["test_ok"],
                    "agent_ok": agent["ok"],
                    "agent_error": agent["error"],
                    "speed_s": round(elapsed, 2),
                    "tokens": agent["tokens"],
                    "cost_usd": agent["cost_usd"],
                    "api_hits": grade["api_hits"],
                    "api_total": grade["api_total"],
                }
                # A refusal / crash is a fail even if leftover files happen to lint.
                if not agent["ok"]:
                    row["passed"] = False
                evaluations.append(row)
                status = "PASS" if row["passed"] else "FAIL"
                print(
                    f"  {status}  {elapsed:.1f}s  tokens={agent['tokens']}  cost={agent['cost_usd']}",
                    file=sys.stderr,
                )
                if not keep:
                    shutil.rmtree(dest, ignore_errors=True)
                write_board(
                    out,
                    model_names,
                    tasks,
                    runs,
                    evaluations,
                    "In progress. Token/cost are null when a CLI did not report them.",
                )

    if dry_run:
        return 0

    write_board(
        out,
        model_names,
        tasks,
        runs,
        evaluations,
        "Measured. Token/cost are null when a CLI did not report them.",
    )
    print(f"wrote {out}", file=sys.stderr)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Agents on Soli eval harness")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--grade-fixture", action="store_true")
    parser.add_argument("--grade", action="store_true")
    parser.add_argument("--task")
    parser.add_argument("--workdir", type=Path)
    parser.add_argument(
        "--models",
        help="comma-separated runners: claude,opencode,grok",
    )
    parser.add_argument(
        "--tasks",
        help="comma-separated task slugs (default: all)",
    )
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--keep", action="store_true", help="keep worktrees under /tmp/soli-evals")
    parser.add_argument("--out", type=Path, default=ROOT / "www" / "data" / "ai_evals.json")
    parser.add_argument("--write-empty", action="store_true")
    args = parser.parse_args()

    if args.list:
        print("tasks:")
        for slug in task_slugs():
            print(f"  {slug}")
        print("runners:")
        for name, spec in RUNNERS.items():
            path = which(spec["bin"]) or "MISSING"
            print(f"  {name:10} {spec['id']:22} model={spec['model']}  bin={path}")
        return 0

    if args.write_empty:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(empty_board(), indent=2) + "\n")
        print(f"wrote empty board to {args.out}")
        return 0

    if args.grade_fixture:
        return grade_fixture()

    if args.grade:
        if not args.task or not args.workdir:
            print("--grade needs --task and --workdir", file=sys.stderr)
            return 2
        dest = args.workdir
        if not dest.exists():
            copy_fixture(dest)
        result = grade_task(args.task, dest)
        print(json.dumps(result, indent=2))
        return 0 if result["passed"] else 1

    if args.models:
        tasks = (
            [t.strip() for t in args.tasks.split(",") if t.strip()]
            if args.tasks
            else task_slugs()
        )
        models = [m.strip() for m in args.models.split(",") if m.strip()]
        return run_board(
            models,
            tasks,
            args.runs,
            args.out,
            args.dry_run,
            args.timeout,
            args.keep,
        )

    parser.print_help()
    return 0


if __name__ == "__main__":
    _ = date
    sys.exit(main())
