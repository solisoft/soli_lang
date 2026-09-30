#!/usr/bin/env python3
"""Agent benchmark: the same coding agent builds the same apps on different stacks.

    bench.py validate                       # acceptance checks vs. the reference server
    bench.py run --stacks soli,rails,next --specs all --trials 3
    bench.py check RUN_DIR/TRIAL --spec todo-api   # re-run the checks on a finished trial
    bench.py image --container docker        # then: bench.py run --container docker ...

Standard library only. See README.md.
"""

import argparse
import concurrent.futures
import datetime
import importlib.util
import json
import os
import secrets
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SPECS = ROOT / "specs"
STACKS = ROOT / "stacks"
RESULTS = ROOT / "results"
# /tmp is a small tmpfs on the dev workstation; node_modules and gems do not fit there.
WORK = Path(os.environ.get("AGENT_BENCH_WORK", Path.home() / ".cache" / "agent-bench"))
PROMPT = (ROOT / "prompt.md").read_text()
SOLIDB_PASSWORD = "agent-bench-password"
LOCKFILES = {"package-lock.json", "Gemfile.lock", "yarn.lock", "pnpm-lock.yaml", "Cargo.lock"}

print_lock = threading.Lock()


def log(message):
    with print_lock:
        print(f"[{datetime.datetime.now():%H:%M:%S}] {message}", flush=True)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for_port(port, timeout, proc=None):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc is not None and proc.poll() is not None:
            return False
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return True
        except OSError:
            time.sleep(0.5)
    return False


def kill_group(proc, grace=5):
    if proc is None or proc.poll() is not None and not _group_alive(proc.pid):
        return
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(proc.pid, sig)
        except ProcessLookupError:
            return
        try:
            proc.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            pass
        if not _group_alive(proc.pid):
            return


def _group_alive(pgid):
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False


# ---------------------------------------------------------------- acceptance


class CheckFailed(Exception):
    pass


class Tester:
    """What a spec's checks.py sees: small HTTP helpers, `expect`, `restart`."""

    def __init__(self, base_url, restart):
        self.base_url = base_url
        self._restart = restart
        self.nonce = secrets.token_hex(3)

    def request(self, method, path, body=None, headers=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.base_url + path, data=data, method=method)
        request.add_header("Accept", "application/json")
        if data is not None:
            request.add_header("Content-Type", "application/json")
        for key, value in (headers or {}).items():
            request.add_header(key, value)
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                status, raw = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        text = raw.decode("utf-8", "replace")
        try:
            return status, json.loads(text) if text.strip() else None
        except json.JSONDecodeError:
            return status, text

    def get(self, path, headers=None):
        return self.request("GET", path, headers=headers)

    def post(self, path, body, headers=None):
        return self.request("POST", path, body, headers)

    def patch(self, path, body, headers=None):
        return self.request("PATCH", path, body, headers)

    def delete(self, path, headers=None):
        return self.request("DELETE", path, headers=headers)

    def expect(self, condition, message):
        if not condition:
            raise CheckFailed(message)

    @staticmethod
    def has_id(record):
        return isinstance(record, dict) and isinstance(record.get("id"), (str, int)) and str(record["id"]) != ""

    @staticmethod
    def is_iso(value):
        if not isinstance(value, str):
            return False
        try:
            datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return True
        except ValueError:
            return False

    def restart(self):
        self._restart()


def load_checks(spec):
    path = SPECS / spec / "checks.py"
    module_spec = importlib.util.spec_from_file_location(f"checks_{spec.replace('-', '_')}", path)
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    return module.CHECKS


def run_checks(spec, server):
    tester = Tester(f"http://127.0.0.1:{server.port}", server.restart)
    results = []
    for check in load_checks(spec):
        try:
            check(tester)
            results.append({"name": check.__name__, "ok": True})
        except CheckFailed as failure:
            results.append({"name": check.__name__, "ok": False, "error": str(failure)[:500]})
        except Exception as error:  # a crash in the app shows up as a broken response
            results.append({"name": check.__name__, "ok": False, "error": f"{type(error).__name__}: {error}"[:500]})
    return results


class HostLauncher:
    """Runs a trial's commands directly on this machine, each in its own process group."""

    def __init__(self, trial_dir):
        self.trial_dir = trial_dir

    def path(self, host_path):
        return str(host_path)

    def popen(self, argv, cwd, env, name, **kwargs):
        # The caller's SOLIDB_* must not leak into a trial: each trial gets its own database.
        base = {k: v for k, v in os.environ.items() if not k.startswith("SOLIDB_")}
        return subprocess.Popen(argv, cwd=cwd, env=dict(base, **env), start_new_session=True, **kwargs)

    def stop(self, proc, name):
        kill_group(proc)

    def close(self):
        pass


class ContainerLauncher:
    """Runs each command in a throwaway container (docker or podman) that sees the trial as /work.

    Host networking, so ports and the acceptance checks work as on the host. soli, solidb and
    claude are the host's binaries, mounted read-only: every stack runs the same agent version.
    Stopping a command removes its container, which also kills whatever it left running.
    """

    TOOLS = ("soli", "solidb", "claude")

    def __init__(self, engine, image, trial_dir):
        self.engine, self.image, self.trial_dir = engine, image, trial_dir
        self.prefix = f"agent-bench-{trial_dir.parent.name}-{trial_dir.name}"
        (trial_dir / "home").mkdir(exist_ok=True)
        self.flags = ["--network", "host", "-v", f"{trial_dir}:/work"]
        if engine == "podman":
            self.flags += ["--userns=keep-id", "--security-opt", "label=disable"]
        else:
            self.flags += ["--user", f"{os.getuid()}:{os.getgid()}"]
        for tool in self.TOOLS:
            if shutil.which(tool):
                self.flags += ["-v", f"{os.path.realpath(shutil.which(tool))}:/usr/local/bin/{tool}:ro"]
        self.base_env = {"HOME": "/work/home", "BUNDLE_PATH": "/work/.bundle", "NEXT_TELEMETRY_DISABLED": "1"}
        if os.environ.get("ANTHROPIC_API_KEY"):
            self.base_env["ANTHROPIC_API_KEY"] = os.environ["ANTHROPIC_API_KEY"]

    def path(self, host_path):
        return str(Path("/work") / Path(host_path).relative_to(self.trial_dir))

    def popen(self, argv, cwd, env, name, **kwargs):
        container = f"{self.prefix}-{name}"
        subprocess.run([self.engine, "rm", "-f", container], capture_output=True)
        env_flags = [flag for key, value in dict(self.base_env, **env).items() for flag in ("-e", f"{key}={value}")]
        command = [self.engine, "run", "--rm", "--name", container, *self.flags, "-w", self.path(cwd),
                   *env_flags, self.image, *argv]
        return subprocess.Popen(command, start_new_session=True, **kwargs)

    def stop(self, proc, name):
        subprocess.run([self.engine, "rm", "-f", f"{self.prefix}-{name}"], capture_output=True, timeout=120)
        kill_group(proc)

    def close(self):
        listed = subprocess.run([self.engine, "ps", "-aq", "--filter", f"name={self.prefix}-"],
                                capture_output=True, text=True).stdout.split()
        if listed:
            subprocess.run([self.engine, "rm", "-f", *listed], capture_output=True, timeout=120)


def make_launcher(opts, trial_dir):
    if getattr(opts, "container", None):
        return ContainerLauncher(opts.container, opts.image, trial_dir)
    return HostLauncher(trial_dir)


class Server:
    """Starts a command, waits for its port, restarts it on demand."""

    def __init__(self, launcher, argv, cwd, env, log_path, boot_timeout, name="server"):
        self.launcher, self.argv, self.cwd, self.env, self.name = launcher, argv, cwd, env, name
        self.log_path, self.boot_timeout = log_path, boot_timeout
        self.port = int(env["PORT"])
        self.proc = None

    def start(self):
        log_file = open(self.log_path, "ab")
        self.proc = self.launcher.popen(self.argv, self.cwd, self.env, self.name, stdout=log_file,
                                        stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        if not wait_for_port(self.port, self.boot_timeout, self.proc):
            raise CheckFailed(f"server did not listen on {self.port} within {self.boot_timeout}s "
                              f"(exit code {self.proc.poll()}), see {self.log_path}")

    def stop(self):
        if self.proc is not None:
            self.launcher.stop(self.proc, self.name)
        deadline = time.time() + 10
        while time.time() < deadline and wait_for_port(self.port, 0.2):
            time.sleep(0.2)

    def restart(self):
        self.stop()
        self.start()


class SoliDB:
    def __init__(self, launcher, data_dir, log_path):
        self.launcher = launcher
        self.port = free_port()
        data_dir.mkdir(parents=True, exist_ok=True)
        # --host explicitly: solidb reads SOLIDB_HOST as its bind address, and dotenv finds it in any
        # parent .env, where it usually holds a client URL (http://localhost:6745) that cannot be bound.
        command = ["solidb", "--host", "127.0.0.1", "--port", str(self.port), "--data-dir", launcher.path(data_dir)]
        self.proc = launcher.popen(command, data_dir, {"SOLIDB_ADMIN_PASSWORD": SOLIDB_PASSWORD}, "solidb",
                                   stdout=open(log_path, "ab"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        if not wait_for_port(self.port, 60, self.proc):
            raise RuntimeError(f"solidb did not start, see {log_path}")

    def env(self):
        return {"SOLIDB_HOST": f"http://127.0.0.1:{self.port}", "SOLIDB_DATABASE": "default",
                "SOLIDB_USERNAME": "admin", "SOLIDB_PASSWORD": SOLIDB_PASSWORD}

    def stop(self):
        self.launcher.stop(self.proc, "solidb")


# ---------------------------------------------------------------- one trial


def sh(command, cwd, env=None, timeout=900):
    return subprocess.run(command, cwd=cwd, env=env, shell=isinstance(command, str), capture_output=True,
                          text=True, timeout=timeout)


def git(app, *args):
    return sh(["git", "-c", "user.name=agent-bench", "-c", "user.email=bench@localhost", *args], app)


def set_env_file(path, values):
    lines = path.read_text().splitlines() if path.exists() else []
    kept = [line for line in lines if line.split("=", 1)[0].strip() not in values]
    path.write_text("\n".join(kept + [f"{key}={value}" for key, value in values.items()]) + "\n")


def code_stats(app, base):
    git(app, "add", "-A")
    numstat = git(app, "diff", "--cached", "--numstat", base).stdout
    added, files = 0, 0
    for line in numstat.splitlines():
        adds, _, name = line.split("\t", 2)
        if Path(name).name in LOCKFILES or adds == "-":
            continue
        added += int(adds)
        files += 1
    return {"lines_added": added, "files_changed": files}


def agent_metrics(stdout):
    try:
        out = json.loads(stdout.strip().splitlines()[-1])
    except (json.JSONDecodeError, IndexError):
        return {"parse_error": True}
    usage = out.get("usage") or {}
    return {
        "cost_usd": out.get("total_cost_usd"),
        "num_turns": out.get("num_turns"),
        "agent_duration_ms": out.get("duration_ms"),
        "is_error": out.get("is_error"),
        "subtype": out.get("subtype"),
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "cache_read_tokens": usage.get("cache_read_input_tokens"),
        "cache_creation_tokens": usage.get("cache_creation_input_tokens"),
    }


def run_trial(run_dir, spec, stack_name, trial, opts):
    stack = json.loads((STACKS / f"{stack_name}.json").read_text())
    trial_dir = run_dir / f"{spec}--{stack_name}--{trial}"
    trial_dir.mkdir(parents=True)
    app = trial_dir / stack["app_dir"]
    record = {"spec": spec, "stack": stack_name, "trial": trial, "model": opts.model,
              "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "dir": str(trial_dir)}
    launcher = make_launcher(opts, trial_dir)
    record["container"] = getattr(opts, "container", None)
    credentials = trial_dir / "home" / ".claude" / ".credentials.json"
    services = []
    server = None
    try:
        env = {}
        if "solidb" in stack["services"]:
            db = SoliDB(launcher, trial_dir / "solidb-data", trial_dir / "solidb.log")
            services.append(db)
            env.update(db.env())

        log(f"{trial_dir.name}: setup — {stack['setup']}")
        setup = launcher.popen(["bash", "-c", stack["setup"]], trial_dir, env, "setup", stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, text=True)
        try:
            setup_log, _ = setup.communicate(timeout=opts.setup_timeout)
        except subprocess.TimeoutExpired:
            launcher.stop(setup, "setup")
            setup_log, _ = setup.communicate()
        (trial_dir / "setup.log").write_text(setup_log)
        if setup.returncode != 0 or not app.is_dir():
            raise RuntimeError(f"setup failed ({setup.returncode}), see setup.log")
        if stack.get("env_file") and "solidb" in stack["services"]:
            set_env_file(app / stack["env_file"], db.env())
        for skill in stack["skills"]:
            source = Path(skill).expanduser()
            shutil.copytree(source, app / ".claude" / "skills" / source.name, dirs_exist_ok=True)
        shutil.copy(SPECS / spec / "SPEC.md", app / "SPEC.md")
        if not (app / ".git").exists():
            git(app, "init", "-q")
        git(app, "add", "-A")
        git(app, "commit", "-q", "--allow-empty", "-m", "agent-bench baseline")
        base = git(app, "rev-parse", "HEAD").stdout.strip()

        prompt = PROMPT.format(stack=stack["label"], setup=stack["setup"], notes=stack["notes"])
        command = ["claude", "-p", prompt, "--output-format", "json", "--model", opts.model,
                   "--setting-sources", "project,local", "--permission-mode", "bypassPermissions",
                   "--no-session-persistence"]
        if opts.budget:
            command += ["--max-budget-usd", str(opts.budget)]
        if not opts.web:
            command += ["--disallowedTools", "WebSearch,WebFetch"]
        if isinstance(launcher, ContainerLauncher) and not os.environ.get("ANTHROPIC_API_KEY"):
            # A fresh copy per trial, removed in `finally`: the container's HOME is inside the trial dir.
            credentials.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(Path.home() / ".claude" / ".credentials.json", credentials)
            credentials.chmod(0o600)
        agent_env = dict(env, PORT=str(free_port()))
        log(f"{trial_dir.name}: agent running (timeout {opts.agent_timeout}s)")
        started = time.time()
        agent = launcher.popen(command, app, agent_env, "agent", stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               stdin=subprocess.DEVNULL, text=True)
        try:
            stdout, stderr = agent.communicate(timeout=opts.agent_timeout)
            record["agent_timed_out"] = False
        except subprocess.TimeoutExpired:
            launcher.stop(agent, "agent")
            stdout, stderr = agent.communicate()
            record["agent_timed_out"] = True
        launcher.stop(agent, "agent")  # servers the agent left running in the background
        record["wall_s"] = round(time.time() - started, 1)
        record["agent_exit"] = agent.returncode
        (trial_dir / "agent.json").write_text(stdout)
        (trial_dir / "agent.stderr").write_text(stderr)
        record.update(agent_metrics(stdout))
        record.update(code_stats(app, base))

        run_sh = app / "run.sh"
        if not run_sh.exists():
            raise CheckFailed("the agent wrote no run.sh")
        server = Server(launcher, ["bash", "run.sh"], app, dict(env, PORT=str(free_port())),
                        trial_dir / "server.log", opts.boot_timeout)
        server.start()
        record["checks"] = run_checks(spec, server)
    except Exception as error:
        record["error"] = f"{type(error).__name__}: {error}"
        if not isinstance(error, (CheckFailed, RuntimeError)):
            record["traceback"] = traceback.format_exc()
    finally:
        if server:
            server.stop()
        for service in services:
            service.stop()
        launcher.close()
        credentials.unlink(missing_ok=True)

    checks = record.get("checks") or [{"name": c.__name__, "ok": False} for c in load_checks(spec)]
    record["checks"] = checks
    record["passed"] = sum(check["ok"] for check in checks)
    record["total"] = len(checks)
    log(f"{trial_dir.name}: {record['passed']}/{record['total']} checks"
        + (f", ${record['cost_usd']:.2f}" if record.get("cost_usd") else "")
        + (f" — {record['error']}" if record.get("error") else ""))
    return record


# ---------------------------------------------------------------- commands


def all_specs():
    return sorted(path.name for path in SPECS.iterdir() if (path / "checks.py").exists())


def pick(value, available, kind):
    chosen = available if value == "all" else value.split(",")
    unknown = sorted(set(chosen) - set(available))
    if unknown:
        sys.exit(f"unknown {kind}: {', '.join(unknown)} (have: {', '.join(available)})")
    return chosen


def cmd_run(opts):
    specs = pick(opts.specs, all_specs(), "spec")
    stacks = pick(opts.stacks, sorted(p.stem for p in STACKS.glob("*.json")), "stack")
    for stack_name in stacks:
        stack = json.loads((STACKS / f"{stack_name}.json").read_text())
        # In a container the image brings the toolchains; soli, solidb and claude still come from here.
        needed = ["claude"] + ([] if opts.container else stack.get("requires", []))
        needed += ["soli", "solidb"] if "solidb" in stack["services"] else []
        for tool in needed:
            if not shutil.which(tool):
                sys.exit(f"stack {stack_name} needs `{tool}` on PATH")
    if opts.container:
        found = subprocess.run([opts.container, "image", "inspect", opts.image], capture_output=True)
        if found.returncode != 0:
            sys.exit(f"no image {opts.image} for {opts.container}: run `bench.py image --container {opts.container}`")
    run_id = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    run_dir = WORK / "runs" / run_id
    run_dir.mkdir(parents=True)
    RESULTS.mkdir(exist_ok=True)
    results_path = RESULTS / f"{run_id}.jsonl"
    jobs = [(spec, stack, trial) for trial in range(1, opts.trials + 1) for spec in specs for stack in stacks]
    log(f"run {run_id}: {len(jobs)} trials, model {opts.model}, {opts.jobs} at a time → {results_path}")
    with concurrent.futures.ThreadPoolExecutor(max_workers=opts.jobs) as pool:
        futures = [pool.submit(run_trial, run_dir, spec, stack, trial, opts) for spec, stack, trial in jobs]
        for future in concurrent.futures.as_completed(futures):
            record = dict(future.result(), run_id=run_id)
            with print_lock, open(results_path, "a") as out:
                out.write(json.dumps(record) + "\n")
    log(f"done — python3 report.py {results_path.relative_to(ROOT)}")


def cmd_validate(opts):
    """Runs every spec's checks against reference/server.py; all must pass."""
    specs = pick(opts.specs, all_specs(), "spec")
    failures = 0
    work = WORK / "validate"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    for spec in specs:
        env = {"PORT": str(free_port()), "REFERENCE_DB": str(work / f"{spec}.sqlite3")}
        server = Server(HostLauncher(work), [sys.executable, str(ROOT / "reference" / "server.py")], work, env,
                        work / f"{spec}.log", 20)
        server.start()
        try:
            results = run_checks(spec, server)
        finally:
            server.stop()
        for result in results:
            mark = "ok  " if result["ok"] else "FAIL"
            print(f"{mark} {spec} :: {result['name']}" + ("" if result["ok"] else f" — {result['error']}"))
            failures += not result["ok"]
    sys.exit(1 if failures else 0)


def cmd_check(opts):
    """Re-runs the checks on a finished trial directory (starts its run.sh again)."""
    trial_dir = Path(opts.trial_dir).resolve()
    app = trial_dir / "app"
    launcher = make_launcher(opts, trial_dir)
    env = {"PORT": str(free_port())}
    db = None
    try:
        if (trial_dir / "solidb-data").exists():
            db = SoliDB(launcher, trial_dir / "solidb-data", trial_dir / "solidb.log")
            env.update(db.env())
            set_env_file(app / ".env", db.env())
        server = Server(launcher, ["bash", "run.sh"], app, env, trial_dir / "server.log", opts.boot_timeout)
        server.start()
        try:
            for result in run_checks(opts.spec, server):
                print(("ok   " if result["ok"] else "FAIL ") + result["name"]
                      + ("" if result["ok"] else f" — {result['error']}"))
        finally:
            server.stop()
    finally:
        if db:
            db.stop()
        launcher.close()


def cmd_image(opts):
    """Builds the image every containerised trial runs in (Ruby, Rails, Node; soli/solidb/claude are mounted)."""
    sys.exit(subprocess.run([opts.container, "build", "-t", opts.image, str(ROOT / "container")]).returncode)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run agent trials")
    run.add_argument("--stacks", default="soli", help="comma-separated, or 'all'")
    run.add_argument("--specs", default="all", help="comma-separated, or 'all'")
    run.add_argument("--trials", type=int, default=1)
    run.add_argument("--model", default="opus")
    run.add_argument("--jobs", type=int, default=1, help="trials in parallel")
    run.add_argument("--budget", type=float, default=10.0, help="max USD per trial (0 = no cap)")
    run.add_argument("--agent-timeout", type=int, default=2700)
    run.add_argument("--boot-timeout", type=int, default=180)
    run.add_argument("--setup-timeout", type=int, default=900)
    run.add_argument("--web", action="store_true", help="let the agent use WebSearch/WebFetch")
    run.set_defaults(func=cmd_run)

    validate = sub.add_parser("validate", help="check the acceptance suites against the reference server")
    validate.add_argument("--specs", default="all")
    validate.set_defaults(func=cmd_validate)

    check = sub.add_parser("check", help="re-run acceptance checks on a finished trial")
    check.add_argument("trial_dir")
    check.add_argument("--spec", required=True)
    check.add_argument("--boot-timeout", type=int, default=180)
    check.set_defaults(func=cmd_check)

    image = sub.add_parser("image", help="build the container image")
    image.set_defaults(func=cmd_image)

    for command in (run, check, image):
        command.add_argument("--container", choices=["docker", "podman"], required=command is image,
                             help="run each trial's commands in a container (see container/Dockerfile)")
        command.add_argument("--image", default="agent-bench:latest")

    opts = parser.parse_args()
    opts.func(opts)


if __name__ == "__main__":
    main()
