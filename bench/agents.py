#!/usr/bin/env python3
"""Benchmark: does Luma help an AI agent find photos? (AI_AGENTS_PLAN.md, step 4)

Runs every task from tasks.json through `claude -p` in three variants:
  none - no Luma at all (baseline: find/grep/Read),
  cli  - SKILL.md + `luma query` on PATH,
  mcp  - `luma mcp` as an MCP server, no skill,
and prints per variant: found the right photo, called Luma by itself, time,
turns, cost.

    python3 bench/agents.py --check          # preflight only, no agent runs
    python3 bench/agents.py                  # every task once, mcp only
    python3 bench/agents.py --variants none,cli,mcp --runs 3   # full comparison
    python3 bench/agents.py --report bench/results/<time>.jsonl  # report again

Every run spends subscription limits like a normal session, hence the cheap
defaults (mcp only, few turns); none is the priciest (the agent Reads images).

Agents get Bash/Read/Glob/Grep without asking (read-only intent, but Bash is
Bash): run it only on your own machine and tasks. Runs are sequential on
purpose: Luma's CLIP and the agent share this laptop's RAM.
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SKILL_DIR = REPO / "agent" / "skills" / "luma"
VARIANTS = ("none", "cli", "mcp")
MCP_TOOL = "mcp__luma__luma_search_images"


def args_parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--tasks", type=Path, default=Path(__file__).with_name("tasks.json"))
    p.add_argument("--photos", type=Path, default=Path("~/Photos").expanduser(),
                   help="root that the expected paths in tasks.json are relative to")
    p.add_argument("--luma", type=Path, default=REPO / "target" / "debug" / "luma",
                   help="luma binary for the cli/mcp variants")
    p.add_argument("--variants", default="mcp", help=f"comma-separated: {', '.join(VARIANTS)}")
    p.add_argument("--runs", type=int, default=1, help="repeats per task and variant")
    p.add_argument("--model", default="sonnet")
    # Explicit: --setting-sources project skips the user's own effortLevel.
    p.add_argument("--effort", default="medium", choices=["low", "medium", "high", "xhigh", "max"])
    # With Luma a photo task takes ~4 turns; the cap stops a lost agent from eating limits.
    p.add_argument("--max-turns", type=int, default=10)
    p.add_argument("--timeout", type=int, default=600, help="seconds per agent run")
    p.add_argument("--out", type=Path, default=REPO / "bench" / "results")
    p.add_argument("--check", action="store_true", help="preflight only")
    p.add_argument("--report", type=Path, metavar="LOG.jsonl",
                   help="print the report of a finished run again, no agent runs")
    return p


# ---------- preflight ----------

def preflight(a, tasks, variants):
    """Every problem at once, so a 40-minute run doesn't die on the first one."""
    problems = []
    if not shutil.which("claude"):
        problems.append("`claude` is not on PATH")
    for t in tasks:
        for rel in t["expect"]:
            if not (a.photos / rel).is_file():
                problems.append(f"task {t['id']}: expected file is missing: {a.photos / rel}")
    if {"cli", "mcp"} & set(variants):
        if not a.luma.is_file():
            problems.append(f"no luma binary at {a.luma} (cargo build, or pass --luma)")
        else:
            problems += luma_problems(a)
    if "cli" in variants and not (SKILL_DIR / "SKILL.md").is_file():
        problems.append(f"no skill at {SKILL_DIR}")
    return problems


def luma_problems(a):
    problems = []
    settings = Path("~/.config/luma/settings.json").expanduser()
    try:
        dirs = [Path(d).expanduser().resolve() for d in json.loads(settings.read_text())["search_dirs"]]
    except (OSError, ValueError, KeyError) as e:
        return [f"can't read search_dirs from {settings}: {e}"]
    photos = a.photos.resolve()
    if not any(photos == d or d in photos.parents for d in dirs):
        problems.append(f"{a.photos} is not inside search_dirs of {settings}: add it and restart Luma")
    # Also proves the daemon is up and CLIP finished indexing.
    r = subprocess.run([a.luma, "query", "clip", "photo"], capture_output=True, text=True)
    if r.returncode != 0:
        problems.append(f"luma query failed: {r.stderr.strip()}")
    elif not any(str(photos) in hit["path"] for hit in json.loads(r.stdout or "[]")):
        # Weak signal only: a generic query may well rank other folders first.
        print(f"note: `luma query clip photo` returned nothing from {a.photos}; "
              "was Luma restarted after the download?", file=sys.stderr)
    return problems


# ---------- one agent run ----------

# shortcut: drops whole PATH dirs that hold a luma (e.g. ~/.local/bin with its
# other tools), upgrade to a filtered shim dir if the baseline misses them.
def path_without_luma():
    dirs = os.environ.get("PATH", "").split(os.pathsep)
    return os.pathsep.join(d for d in dirs if not (Path(d) / "luma").exists())


def claude_cmd(a, variant, prompt, workdir):
    cmd = [
        "claude", "-p", prompt,
        "--model", a.model,
        "--effort", a.effort,
        "--output-format", "stream-json", "--verbose",
        "--max-turns", str(a.max_turns),
        # Only the temp workdir's .claude/: no user hooks, plugins or MCP servers.
        "--setting-sources", "project",
        "--strict-mcp-config",
        "--no-session-persistence",
        # Same reach in every variant: the whole home, like a session started in ~.
        "--add-dir", str(Path.home()),
        "--permission-mode", "dontAsk",
        "--allowedTools", "Bash", "Read", "Glob", "Grep",
    ]
    if variant == "mcp":
        servers = {"mcpServers": {"luma": {"command": str(a.luma), "args": ["mcp"]}}}
        cmd += [MCP_TOOL, "--mcp-config", json.dumps(servers)]
    return cmd


def run_agent(a, variant, task):
    # No "luma" in the name: the init message carries cwd, see setup_ok.
    with tempfile.TemporaryDirectory(prefix="agent-bench-") as workdir:
        env = dict(os.environ, PATH=path_without_luma())
        if variant == "cli":
            shutil.copytree(SKILL_DIR, Path(workdir, ".claude", "skills", "luma"))
            bin_dir = Path(workdir, "bin")
            bin_dir.mkdir()
            (bin_dir / "luma").symlink_to(a.luma.resolve())
            env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
        started = time.monotonic()
        try:
            proc = subprocess.run(claude_cmd(a, variant, task["prompt"], workdir), cwd=workdir, env=env,
                                  capture_output=True, text=True, timeout=a.timeout)
            lines, stderr = proc.stdout.splitlines(), proc.stderr
        except subprocess.TimeoutExpired as e:
            lines, stderr = (e.stdout or b"").decode(errors="replace").splitlines(), "timeout"
    return grade(task, variant, parse_stream(lines), stderr, time.monotonic() - started)


def parse_stream(lines):
    """stream-json: init, assistant/user messages, final result."""
    init, result, tools = {}, {}, []
    for line in lines:
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("type") == "system" and msg.get("subtype") == "init":
            init = msg
        elif msg.get("type") == "assistant":
            tools += [c for c in msg["message"].get("content", []) if c.get("type") == "tool_use"]
        elif msg.get("type") == "result":
            result = msg
    return init, result, tools


def used_luma(tool):
    return tool["name"] == MCP_TOOL or (
        tool["name"] == "Bash" and "luma query" in tool.get("input", {}).get("command", ""))


def grade(task, variant, parsed, stderr, wall):
    init, result, tools = parsed
    answer = result.get("result") or ""
    answer = answer.replace("~/", f"{Path.home()}/")
    # Setup sanity: the variant really had (or lacked) Luma. The init message
    # lists tools, MCP servers and skills; "luma" anywhere in it is enough.
    has_luma = "luma" in json.dumps(init).lower()
    return {
        "task": task["id"],
        "variant": variant,
        "prompt": task["prompt"],
        "expect": task["expect"],
        "control": not task["expect"],
        # Expected paths are relative to the photos root, so a substring hit
        # works for absolute and ~/ answers alike.
        "found": any(rel in answer for rel in task["expect"]),
        "luma_calls": sum(map(used_luma, tools)),
        "reads": sum(t["name"] == "Read" for t in tools),
        "tool_calls": len(tools),
        "turns": result.get("num_turns"),
        "seconds": round(result.get("duration_ms", wall * 1000) / 1000, 1),
        "cost_usd": result.get("total_cost_usd", 0.0),
        "setup_ok": bool(init) and has_luma == (variant != "none"),
        "error": None if result and not result.get("is_error") else (stderr.strip()[-500:] or "no result"),
        "answer": answer[-1000:],
    }


# ---------- report ----------

def broken(r):
    return bool(r["error"]) or not r["setup_ok"]


def outcome(r):
    if broken(r):
        return "ERR"
    if r["control"]:
        return "ok" if r["luma_calls"] == 0 else "LUMA"
    return "ok" if r["found"] else "MISS"


def bar(hits, total, width=20):
    filled = round(width * hits / total) if total else 0
    return "█" * filled + "░" * (width - filled)


def one_line(text, width=110):
    text = " ".join(text.split())
    return text if len(text) <= width else "…" + text[-width:]


def report(rows, color):
    paint = (lambda s, code: f"\033[{code}m{s}\033[0m") if color else (lambda s, code: s)
    GREEN, RED, DIM, BOLD = "32", "31", "2", "1"
    first = rows[0]
    out = [paint(f"━━ Luma agent bench · {first.get('model')} / {first.get('effort')} · "
                 f"{len({r['task'] for r in rows})} tasks · {len(rows)} runs ━━", BOLD)]

    for v in dict.fromkeys(r["variant"] for r in rows):
        rs = [r for r in rows if r["variant"] == v]
        photo = [r for r in rs if not r["control"]]
        control = [r for r in rs if r["control"]]

        def rate(label, hits, total, good):
            pct = hits / total if total else 0
            return (f"  {label:<20}{paint(bar(hits, total), GREEN if good(pct) else RED)}"
                    f"  {hits:>3}/{total:<3} {pct:>4.0%}")

        secs = sorted(r["seconds"] for r in rs)
        cost = sum(r["cost_usd"] for r in rs)
        n_broken = sum(map(broken, rs))
        out += [
            "",
            paint(f"▌ {v}", BOLD),
            rate("found the photo", sum(r["found"] for r in photo), len(photo), lambda p: p >= 0.8),
            rate("called Luma itself", sum(r["luma_calls"] > 0 for r in photo), len(photo), lambda p: p >= 0.8),
            rate("Luma on controls", sum(r["luma_calls"] > 0 for r in control), len(control), lambda p: p == 0),
            f"  {'time':<20}median {statistics.median(secs):.1f} s · max {secs[-1]:.1f} s",
            f"  {'turns':<20}median {statistics.median(r['turns'] or 0 for r in rs):g}",
            f"  {'images opened':<20}{statistics.fmean(r['reads'] for r in rs):.1f} Read per run",
            f"  {'cost':<20}${cost:.2f} total · ${cost / len(rs):.3f} per run",
            f"  {'broken runs':<20}" + paint(str(n_broken), RED if n_broken else GREEN),
        ]
        # What to look at: every run that didn't go as expected, with the reason.
        for r in rs:
            o = outcome(r)
            if o == "ok":
                continue
            if o == "MISS":
                why = f"expected {' | '.join(r['expect'])}\n{'':8}answer: {one_line(r['answer'])}"
            elif o == "LUMA":
                why = f"called Luma on: {r['prompt']}"
            else:
                why = one_line(r["error"] or "setup broken: Luma (not) in the agent's init, see log")
            out.append(f"  {paint(f'✗ {o:<4}', RED)} {r['task']:<18} " + paint(why, DIM))
    return "\n".join(out)


def main():
    a = args_parser().parse_args()
    if a.report:
        rows = [json.loads(line) for line in a.report.read_text().splitlines() if line]
        print(report(rows, sys.stdout.isatty()))
        return
    variants = [v for v in a.variants.split(",") if v]
    if unknown := set(variants) - set(VARIANTS):
        sys.exit(f"unknown variants: {', '.join(unknown)} (available: {', '.join(VARIANTS)})")
    tasks = json.loads(a.tasks.read_text())

    problems = preflight(a, tasks, variants)
    for p in problems:
        print(f"preflight: {p}", file=sys.stderr)
    if problems:
        sys.exit(1)
    print(f"preflight ok: {len(tasks)} tasks, variants {variants}, {a.runs} run(s), "
          f"model {a.model}, effort {a.effort}")
    if a.check:
        return

    a.out.mkdir(parents=True, exist_ok=True)
    log = a.out / time.strftime("%Y%m%d-%H%M%S.jsonl")
    rows = []
    total = len(tasks) * len(variants) * a.runs
    for _ in range(a.runs):
        for task in tasks:
            # Variants interleaved per task: any drift (cache, load) hits all alike.
            for v in variants:
                row = run_agent(a, v, task) | {"model": a.model, "effort": a.effort}
                rows.append(row)
                with log.open("a") as f:
                    f.write(json.dumps(row, ensure_ascii=False) + "\n")
                print(f"[{len(rows):>3}/{total}] {v:<4} {task['id']:<18} {outcome(row):<4} "
                      f"luma×{row['luma_calls']}  {row['seconds']:>5.1f}s  ${row['cost_usd']:.3f}")
    text = report(rows, sys.stdout.isatty())
    print("\n" + text)
    # Plain copy next to the log, for diffs between runs.
    log.with_suffix(".txt").write_text(report(rows, color=False) + "\n")
    print(f"\nlog: {log}\nreport: {log.with_suffix('.txt')}")


if __name__ == "__main__":
    main()
