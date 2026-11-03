#!/usr/bin/env python3
"""Differential grid for a `//` over a caught foreign-`$x` `foreach` that is the UPDATE of a `foreach`, with a non-literal
alternate (#5156).

#5114 read a literal alternate (`// 5`); the state (`// .`), a variable (`// $x`) and the value heads fall back the same way, at
the register the `//` entered with, and this is the grid that gated reading them. The neighbouring
`jq-foreach-update-null-marker-sweep.py` (#5205) covers the sole nested `foreach` UPDATE.

Grid (sampled at random, deduplicated): `$x` bind x outer source x outer INIT x nested pattern x nested body x nested INIT/UPDATE/
EXTRACT x left operand (`FE`, `FE | .`, `try FE`, `FE?`, `1 | FE`, `.a | FE`, `FE | null`) x alternate (`.`, `$x`, `5`, `null`,
`false`, `(5,6)`, `{b:1}`, `[1]`, `.a`, `empty`, `error("z")`, `$i`, `"s"`, `(.,.)`, `$y`) x optional `| .` x outer EXTRACT x write
context (`path`, `?`, `try .. catch`, `|=`, `del`) x document.

Rows are compared against `/usr/bin/jq` on stdout and exit status. `fixed`: wrong on `--base`, matching on `--bin`. `regress`: matching on
`--base`, not on `--bin`. `newwrong`: wrong on both, and differently (a prefix of jq's paths before a refusal is one). Exit status 1 on any
`regress`. Run `--base` as a build from before the change.
`--jq-gate` also fails the run on any row where `--bin` differs from jq on stdout or exit status (`jqwrong`), and
is the only judgement when `--base` is not given. No residual is documented today, and it is how
`succinctly dev compare jq --dimension sweeps` runs this script (#5552); a residual that becomes accepted has to
be listed here first.

Usage: scripts/jq-foreach-update-alt-alternate-sweep.py --bin BIN (--base BIN | --jq-gate) [--jq JQ] [--n N] [--seed S] [--show N] [-v]
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
docs=['{"a":null}','{"a":{"a":null}}','{"a":true}','{"a":1}','{"a":[null]}','{"a":{"b":1}}','null','{"a":{"a":{"a":null}}}','{"a":false}','[null]']
xbind=['. as $x','. as $x','.a as $x','.a? as $x']
outer_src=['(.a,.)','(.,.a)','(.a,.a,.)','(.a)','(.,.a,.)','(.[]?)','(..)','(.a?,.)']
outer_init=['.','.','null','0']
pats=['{a:$a}','{a:$a}','[$a]','{c:$a}','{a:{a:$a}}']
nested_init=['.','.','null','0']
nested_upd=['.','.','$y']
nested_ext=['.','.','$y']
body=['.','.','1']
alts=['.','.','$x','5','null','false','(5,6)','{b:1}','[1]','.a','empty','error("z")','$i','"s"','(.,.)','$y']
fe_wrap=['%s','(%s)|.','try (%s)','(%s)?','(1|%s)','(.a|%s)','(%s)|null']
outer_ext=['$i','.','$i','$i|.','null']
ctx=['[path({})?]','try path({}) catch 5','[path({})?]','path({})','({}) = 9','del({})']
def mk():
    p=random.choice(pats)
    src='(try ($x as %s | %s))'%(p,random.choice(body))
    nf='foreach %s as $y (%s; %s; %s)'%(src,random.choice(nested_init),random.choice(nested_upd),random.choice(nested_ext))
    left=random.choice(fe_wrap)%nf
    alt=random.choice(alts)
    upd='(%s) // %s'%(left,alt)
    if random.random()<.25: upd='(%s)|.'%upd
    prog='%s | foreach %s as $i (%s; %s; %s)'%(random.choice(xbind),random.choice(outer_src),random.choice(outer_init),upd,random.choice(outer_ext))
    f=random.choice(ctx).replace('{}',prog)
    return random.choice(docs), f
rows=list(dict.fromkeys(mk() for _ in range(N)))
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
sys.exit(1 if (args.jq_gate and cnt['jqwrong']) or cnt['regress'] else 0)
