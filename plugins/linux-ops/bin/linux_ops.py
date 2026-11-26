#!/usr/bin/env python3
"""Linux operations for LocalHarness; runs host commands from the Flatpak."""
import json, os, subprocess, sys, urllib.request

def host(cmd, timeout=20):
    if os.environ.get("FLATPAK_ID"):
        cmd = ["flatpak-spawn", "--host", *cmd]
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)

def log_check(unit):
    since = "24 hours ago"
    status = host(["systemctl", "--user", "show", unit, "-p", "ActiveState", "-p", "NRestarts", "-p", "ActiveEnterTimestamp", "--no-pager"])
    recent = host(["journalctl", "--user", "-u", unit, "--since", since, "-n", "120", "--no-pager"])
    warnings = host(["journalctl", "--user", "-u", unit, "--since", since, "-p", "warning", "-n", "200", "--no-pager"])
    status_text = status.stdout.strip()
    recent_lines = recent.stdout.splitlines()
    app_errors = [line for line in recent_lines if not (" W " in line or "warning" in line.lower()) and any(x in line.lower() for x in ("error", "failed", "exceed", "panic"))]
    app_warnings = [line for line in recent_lines if " W " in line or "warning" in line.lower()]
    active = "ActiveState=active" in status_text
    restarts = 0
    for line in status_text.splitlines():
        if line.startswith("NRestarts="):
            try: restarts = int(line.split("=", 1)[1])
            except ValueError: pass
    if not active: verdict = "FAILED"
    elif restarts or app_errors or app_warnings: verdict = "DEGRADED"
    else: verdict = "OK"
    if verdict == "FAILED":
        report = f"The systemd user unit `{unit}` is not active. Verdict: FAILED. NRestarts={restarts}."
    elif verdict == "DEGRADED":
        report = f"The systemd user unit `{unit}` is active, with NRestarts={restarts}. Verdict: DEGRADED."
        if app_errors:
            report += f" {len(app_errors)} application error marker(s) were found.\nRecent application errors:\n"
            report += "\n".join("- " + line.strip()[:240] for line in app_errors[:3]) + "\n"
        if app_warnings:
            report += f" {len(app_warnings)} application warning marker(s) were found.\nRecent application warnings:\n"
            report += "\n".join("- " + line.strip()[:240] for line in app_warnings[:3]) + "\n"
        report += "Inspect details if these recur; do not restart unless the unit itself is failing."
    else:
        report = f"The systemd user unit `{unit}` is active, with NRestarts={restarts}. Verdict: OK. No application error markers or journal warnings were found in the selected window ({since})."
    print("REPORT\n" + report)

def man_page(command):
    result = host(["man", "-P", "cat", command], 30)
    if result.returncode == 0 and result.stdout.strip():
        print(f"SOURCE: man {command}\n\n{result.stdout.strip()}")
        return
    result = host([command, "--help"], 20)
    print(f"SOURCE: {command} --help\n\n{result.stdout.strip() or result.stderr.strip()}")

def model_status():
    for path in ("/v1/models", "/props"):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:8080{path}", timeout=5) as response:
                print(path, response.read().decode())
        except Exception as error:
            print(path, "error:", error)

def service(action):
    result = host(["systemctl", "--user", action, "llama-server"], 120)
    print(result.stdout.strip() or result.stderr.strip())

if __name__ == "__main__":
    action = sys.argv[1]
    if action == "log-check": log_check(sys.argv[2])
    elif action == "man-page": man_page(sys.argv[2])
    elif action == "model-status": model_status()
    elif action == "model-load": service("start")
    elif action == "model-unload": service("stop")
    else: raise SystemExit(f"unknown action {action}")
