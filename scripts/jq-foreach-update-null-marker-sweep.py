#!/usr/bin/env python3
"""Differential grid for a caught destructure of a `null`, boolean or container `$x` in the sole nested `foreach` of a
`foreach` UPDATE (#5205).

A `null` or boolean `$x` is `jv_identical` by kind to the register of a `null` element, so the nested source's pattern step
passes there and moves jq's register onto the bound child; on any other element it fails `path_intact` and the `try` leaves
the source empty. #5189/#5113 read a container `$x` only, because the passing step was placed wrongly; #5205 places it (a
literal nested INIT, `bare_bind_foreach_update_key`) and reads the null/boolean marker for the UPDATEs it places. This is the
grid that gated it; the neighbouring `jq-foreach-update-if-arm-sweep.py` (#5115) covers the `if` UPDATE.

Grid (sampled at random, deduplicated): `$x` bind (`. as $x`, `.a as $x`, `.a.a as $x`, `.b as $x`, `.a? as $x`) x outer source
x outer INIT x nested pattern (flat, array, two-level) x nested source (`try` or not; bare `$x as P | BODY`, the pipe
spelling `$x | . as P | .`) x nested INIT (`.`, `null`, `0`, `false`, `true`, `.a?`) x nested UPDATE/EXTRACT x UPDATE wrapper
(none, `| .`, `try`, `?`, `try .. catch $i`, `| null`, `if`, `//`, comma siblings, a literal or navigated head) x outer
EXTRACT x write context (`path`, `?`, `try .. catch`, `|=`, `del`) x document (null-, boolean- and container-bearing).

Rows are compared against `/usr/bin/jq` on stdout and exit status. `fixed`: wrong on `--base`, matching on `--bin`.
`regress`: matching on `--base`, not on `--bin`. `newwrong`: wrong on both, and differently. Exit status 1 on any
`regress` or `newwrong`. Run `--base` as a build from before the change.
`--jq-gate` also fails the run on any row where `--bin` differs from jq on stdout or exit status (`jqwrong`), and
is the only judgement when `--base` is not given. No residual is documented today, and it is how
`succinctly dev compare jq --dimension sweeps` runs this script (#5552); a residual that becomes accepted has to
be listed here first.

Usage: scripts/jq-foreach-update-null-marker-sweep.py --bin BIN (--base BIN | --jq-gate) [--jq JQ] [--n N] [--seed S] [--show N] [-v]
"""
import argparse
import concurrent.futures as cf
import random
import subprocess
import sys

ap = argparse.ArgumentParser()
ap.add_argument("--bin", required=True)
ap.add_argument("--base")
ap.add_argument("--jq-gate", action="store_true",
                help="also fail on any row where --bin differs from jq (the gate without --base, #5552)")
ap.add_argument("--jq", default="/usr/bin/jq")
ap.add_argument("--n", type=int, default=40000)
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--show", type=int, default=40)
ap.add_argument("--jobs", type=int, default=8)
ap.add_argument("-v", action="store_true")
args = ap.parse_args()
if not args.base and not args.jq_gate:
    ap.error('pass --base to judge a change against a build, or --jq-gate to judge --bin against jq alone')
BASE, NEW, N = args.base, args.bin, args.n
random.seed(args.seed)
docs=['{"a":null}','{"a":{"a":null}}','{"a":true}','{"a":false}','{"a":{"a":true}}','{"a":[null]}','{"a":{"a":{"a":null}}}','{"a":1,"b":null}','null','{"a":{"b":1}}','{"a":[1]}','{"a":{"a":false}}','[null]']
xbind=['. as $x','.a as $x','.a as $x','.a.a as $x','.b as $x','.a? as $x']
outer_src=['(.a)','(.a,.)','(.,.a)','(.a,.a,.)','(.,.a,.a)','(.a?,.)','(.a,.a.a,.)','(.[]?)','(..)','(.a,.b)']
outer_init=['.','.','.','null','0','false']
pats=['{a:$a}','{a:$a}','{c:$a}','[$a]','{a:{a:$a}}']
nested_init=['.','null','0','false','true','.a?']
nested_upd=['.','.','$y','1','empty']
nested_ext=['.','.','$y','1']
body=['.','.','.','1','$x']
upd_wrap=['%s','%s','(%s)|.','try (%s)','(%s)?','try (%s) catch $i','(%s)|null','if . then %s else . end','(%s)//5','(%s, empty)','(%s, 7)','(1 | %s)','(.a | %s)']
outer_ext=['$i','.','$i','$x']
ctx=['[path({})?]','try path({}) catch 5','[path({})?]','path({})','({}) = 9','del({})']
def mk():
    p=random.choice(pats)
    r = random.random()
    if r < .1:
        src = '(try ($x | . as %s | .))' % p
    elif r < .8:
        src = '(try ($x as %s | %s))' % (p, random.choice(body))
    else:
        src = '($x as %s | %s)' % (p, random.choice(body))
    nf='foreach %s as $y (%s; %s; %s)'%(src,random.choice(nested_init),random.choice(nested_upd),random.choice(nested_ext))
    upd=random.choice(upd_wrap)%nf
    prog='%s | foreach %s as $i (%s; %s; %s)'%(random.choice(xbind),random.choice(outer_src),random.choice(outer_init),upd,random.choice(outer_ext))
    f=random.choice(ctx).replace('{}',prog)
    return random.choice(docs), f
rows=list({mk() for _ in range(N)})
def run(cmd, inp, f):
    try:
        p=subprocess.run(cmd+['-c',f],input=inp.encode(),capture_output=True,timeout=20)
        err=p.stderr.decode().strip().splitlines()
        e=err[0].split('): ',1)[-1] if err else ''
        return (p.stdout.decode(), p.returncode, e)
    except subprocess.TimeoutExpired: return ('TIMEOUT',-1,'')
def one(r):
    inp,f=r
    return r, run([args.jq],inp,f), (run([BASE,'jq'],inp,f) if BASE else None), run([NEW,'jq'],inp,f)
cnt={'ok':0,'fixed':0,'regress':0,'bothbad':0,'newwrong':0,'jqwrong':0}
out=[]
with cf.ThreadPoolExecutor(args.jobs) as ex:
    for r,j,b,n in ex.map(one,rows):
        nm2=(n[:2]==j[:2])
        # A row jq itself times out on (load, not the row) is no evidence against --bin.
        if not nm2 and j[0]!='TIMEOUT': cnt['jqwrong']+=1
        if b is None:
            # No base: the row is judged against jq alone (--jq-gate).
            cnt['ok' if nm2 else 'bothbad']+=1
            if not nm2: out.append(('JQWRONG',r,j,None,n))
            continue
        bm2=(b[:2]==j[:2])
        if bm2 and not nm2: k='regress'
        elif nm2 and not bm2: k='fixed'
        elif nm2 and bm2: k='ok'
        else: k='bothbad'
        if k=='bothbad' and b[:2]!=n[:2]:
            cnt['newwrong']+=1
            out.append(('NEWWRONG',r,j,b,n))
        cnt[k]+=1
        if k in('regress','fixed') or (k=='bothbad' and args.v): out.append((k,r,j,b,n))
for k,r,j,b,n in out[:args.show]:
    print(f'{k.upper()}: {r[0]} | {r[1]}\n   jq  : {j}\n   base: {b}\n   new : {n}')
print(cnt, 'rows', len(rows))
sys.exit(1 if (args.jq_gate and cnt['jqwrong']) or cnt['regress'] or cnt['newwrong'] else 0)
