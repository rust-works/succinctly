#!/usr/bin/env python3
"""cap.py SECONDS CMD...: run CMD with RLIMIT_CORE (1,1), in its own process group, and kill that
group (by its id, never by pattern) if it runs past SECONDS. Exit status is CMD's, or 124 on the cap."""
import os, resource, signal, subprocess, sys

cap = float(sys.argv[1])
p = subprocess.Popen(sys.argv[2:], start_new_session=True,
                     preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (1, 1)))
try:
    sys.exit(p.wait(timeout=cap))
except subprocess.TimeoutExpired:
    sys.stderr.write(f"cap.py: {sys.argv[2:]} passed its {cap:.0f}s cap; killing group {p.pid}\n")
    os.killpg(p.pid, signal.SIGTERM)
    try:
        p.wait(timeout=30)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
    sys.exit(124)
