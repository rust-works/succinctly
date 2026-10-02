#!/usr/bin/env python3
"""Oracle sweep for the local-zone date builtins (#3054).

Runs `localtime`, `localtime | mktime` and `strflocaltime`'s `%z`, `%Z` and `%s`
(for a number and for the broken-down array `localtime` returns) through
`succinctly jq` and through a reference jq, for a matrix of `TZ` values (IANA
names, the system zone, POSIX strings with and without a rule, empty and
invalid values) and timestamps (both hemispheres' daylight-time edges, a zone
whose offset has changed, dates before 1900, fractional seconds), and reports
how often the two agree, per `TZ` category and column. `--truth` adds an
independent reference for the IANA zones, Python's `zoneinfo`.

What to expect, and why it is a report and not a gate:

* `localtime` and `mktime(localtime)` for IANA names and the system zone agree
  with every jq tried (1.7.1 and 1.8.2, macOS and glibc) on every timestamp
  within +-10^12 seconds.
* `%z`, `%Z` and `%s` are the zone's own for the instant. jq 1.7.1 labels a
  daylight-time instant with the zone's standard offset and name (and `+0000`
  for `%z` on glibc), so the agreement with 1.7.1 is low there *by design*
  (docs/compliance/jq/limitations.md); jq 1.8.2 prints the true values, so run
  it against 1.8.2 (`JQ_ORACLE=/opt/homebrew/bin/jq`) to see agreement near
  100%, and `--truth` to see which of the two is right.
* `strflocaltime` of a number past 2^63 aborts jq 1.7.1 (exit 134), so those
  timestamps are only asked of `localtime`.

usage:
    cargo build --features cli
    scripts/jq-localtime-oracle-sweep.py [--truth] [--show N] [--check-fields] [binary]

`binary` defaults to target/debug/succinctly under the repository root. The
oracle is /usr/bin/jq unless `JQ_ORACLE` names another jq. Every divergence is
written to `.ai/scratch/localtime-divergences.txt`; the first N (default 0) are
echoed. `--check-fields` exits 1 if `localtime` or `mktime(localtime)` differs
from the oracle for an IANA name or the system zone within +-10^12 seconds. The
`TZ` environment of the caller is not used: every run sets or unsets it.
"""
import collections
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORACLE = os.environ.get("JQ_ORACLE", "/usr/bin/jq")

# (category, TZ). None is "TZ unset": the system zone.
TZS = (
    [("A  IANA name / system zone", tz) for tz in (
        None, "Asia/Tokyo", "America/New_York", "Australia/Sydney", "Europe/London", "Asia/Kolkata",
        "Pacific/Chatham", "Asia/Kathmandu", "Australia/Lord_Howe", "America/St_Johns", "Europe/Dublin",
        "Pacific/Apia", "America/Sao_Paulo", "Africa/Casablanca", "Etc/GMT+5", "UTC", "GMT", ":Asia/Tokyo")]
    + [("B  POSIX offset string", tz) for tz in ("EST5", "PST8", "UTC-9", "JST-9", "<+0530>-5:30", "XXX-24")]
    + [("C  POSIX string with DST", tz) for tz in (
        "EST5EDT", "XYZ5XYD", "NZST-12NZDT,M9.5.0,M4.1.0/3", "EST5EDT,M3.2.0,M11.1.0")]
    + [("D  empty / unrecognised", tz) for tz in ("", "Foo/Bar", "Asia/Tokyo ", "XXX-25", "XXX99", "garbage", "5")]
)
# Within +-10^12 s ("ordinary") and beyond it.
TS = [
    "0", "1", "-1", "1720000000", "1705000000", "1700000000", "1710053999", "1710054000",
    "1730613599", "1730613600", "1711846799", "1711846800", "1712419199", "1712419200",
    "-2000000000", "4000000000", "2000000000", "2147483647", "2147483648", "4102444800",
    "-2147483648", "-2147483649", "-3000000000", "-5000000000", "-9000000000", "10000000000",
    "253402300800", "1720000000.75", "-0.5", "-1.5",
]
FAR = ["1000000000000", "10000000000000", "1000000000000000", "-10000000000000", "1e15", "-1e15"]
# jq 1.7.1 aborts on `strflocaltime` of these; they are only asked of `localtime`.
HUGE = ["9223372036854775807", "-9223372036854775808", "1e19", "-1e19", "67768036191676799"]

COLS = ["localtime", "mktime(localtime)", "%z", "%Z", "%s", "arr %z", "arr %Z", "arr %s"]
PROG = r"""[.[] | . as $t | [
  (try ($t|localtime) catch "ERR"),
  (try ($t|localtime|mktime) catch "ERR"),
  (try ($t|strflocaltime("%z")) catch "ERR"),
  (try ($t|strflocaltime("%Z")) catch "ERR"),
  (try ($t|strflocaltime("%s")) catch "ERR"),
  (try ($t|localtime|strflocaltime("%z")) catch "ERR"),
  (try ($t|localtime|strflocaltime("%Z")) catch "ERR"),
  (try ($t|localtime|strflocaltime("%s")) catch "ERR")
]][]"""
PROG_HUGE = r"""[.[] | . as $t | [(try ($t|localtime) catch "ERR"), (try ($t|localtime|mktime) catch "ERR")]][]"""


def run(cmd, tz, prog, items):
    env = {k: v for k, v in os.environ.items() if k != "TZ"}
    if tz is not None:
        env["TZ"] = tz
    p = subprocess.run(cmd + ["-c", prog], input="[" + ",".join(items) + "]", capture_output=True,
                       text=True, env=env, timeout=300)
    if p.returncode != 0:
        sys.exit(f"{cmd[0]} failed for TZ={tz!r} (exit {p.returncode}): {p.stderr.strip()[:200]}")
    return [json.loads(line) for line in p.stdout.splitlines()]


def fmt_z(off):
    sign = "-" if off < 0 else "+"
    minutes = abs(int(off)) // 60
    return f"{sign}{minutes // 60:02d}{minutes % 60:02d}"


def truth(zone, ts):
    """Python's zoneinfo: [localtime fields, %z, %Z, %s] for an integer instant."""
    from datetime import datetime
    from zoneinfo import ZoneInfo
    d = datetime.fromtimestamp(ts, ZoneInfo(zone))
    return [[d.year, d.month - 1, d.day, d.hour, d.minute, d.second], fmt_z(d.utcoffset().total_seconds()),
            d.tzname(), ts]


def main():
    args = sys.argv[1:]
    show = int(args[args.index("--show") + 1]) if "--show" in args else 0
    if "--show" in args:
        del args[args.index("--show"):args.index("--show") + 2]
    with_truth = "--truth" in args
    check = "--check-fields" in args
    args = [a for a in args if not a.startswith("--")]
    binary = [args[0] if args else os.path.join(ROOT, "target/debug/succinctly"), "jq"]

    agree = collections.Counter()
    total = collections.Counter()
    divergences = []
    fails = 0
    truth_ok = collections.Counter()
    truth_total = collections.Counter()
    iana = [tz for cat, tz in TZS if cat.startswith("A") and tz not in (None, ":Asia/Tokyo", "UTC", "GMT", "Etc/GMT+5")]
    for cat, tz in TZS:
        for items, prog, cols, scope in ((TS, PROG, COLS, "ordinary"), (FAR, PROG, COLS, "far"),
                                         (HUGE, PROG_HUGE, COLS[:2], "far")):
            want = run([ORACLE], tz, prog, items)
            got = run(binary, tz, prog, items)
            for i, item in enumerate(items):
                for c, col in enumerate(cols):
                    key = (cat, col, scope)
                    total[key] += 1
                    if want[i][c] == got[i][c]:
                        agree[key] += 1
                    else:
                        divergences.append(f"TZ={tz!r} {item} {col}: succinctly {got[i][c]!r} / oracle {want[i][c]!r}")
                        if check and cat.startswith("A") and scope == "ordinary" and c < 2:
                            fails += 1
            if with_truth and tz in iana and scope == "ordinary":
                # Whole seconds zoneinfo can place (year 1..9999), from 1900: succinctly's `%s` rule
                # before 1900 is a documented macOS-modelled divergence, not the zone's.
                ints = [i for i in items if "." not in i and "e" not in i and -2208988800 <= int(i) < 253402300800]
                out = run(binary, tz, PROG, ints)
                for i, item in enumerate(ints):
                    t = truth(tz, int(item))
                    for name, a, b in (("localtime", t[0], out[i][0][:6]), ("%z", t[1], out[i][2]),
                                       ("%Z", t[2], out[i][3]), ("%s", str(t[3]), out[i][4])):
                        truth_total[name] += 1
                        truth_ok[name] += a == b

    print(f"binary: {binary[0]}\noracle: {ORACLE} ({subprocess.run([ORACLE, '--version'], capture_output=True, text=True).stdout.strip()})\n")
    for scope in ("ordinary", "far"):
        print(f"agreement with the oracle, timestamps {'within' if scope == 'ordinary' else 'beyond'} +-10^12 s (matches/rows)")
        print(f"  {'category':28}" + "".join(f"{c:>19}" for c in COLS))
        for cat in sorted({c for c, _ in TZS}):
            cells = []
            for col in COLS:
                n = total[(cat, col, scope)]
                cells.append(f"{agree[(cat, col, scope)]}/{n} ({100 * agree[(cat, col, scope)] / n:.0f}%)" if n else "-")
            print(f"  {cat:28}" + "".join(f"{x:>19}" for x in cells))
        print()
    if with_truth:
        print(f"agreement with the real zone (Python zoneinfo), {len(iana)} IANA zones, ordinary integer instants")
        for name in ("localtime", "%z", "%Z", "%s"):
            print(f"  {name:10} {truth_ok[name]}/{truth_total[name]} ({100 * truth_ok[name] / truth_total[name]:.1f}%)")
        print()
    os.makedirs(os.path.join(ROOT, ".ai/scratch"), exist_ok=True)
    path = os.path.join(ROOT, ".ai/scratch/localtime-divergences.txt")
    with open(path, "w") as f:
        f.write("\n".join(divergences) + "\n")
    print(f"{len(divergences)} divergences written to {path}")
    for line in divergences[:show]:
        print("  " + line)
    if check and fails:
        print(f"--check-fields: {fails} localtime/mktime divergences for an IANA name or the system zone")
        sys.exit(1)


main()
