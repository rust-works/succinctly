#!/usr/bin/env python3
"""Differential sweep for #6402: random streams of lines with `%`, `@` and a backtick starting a node.

    scripts/yq-directive-in-document-sweep.py SEED COUNT BASELINE SUBJECT

Each stream is two to five lines drawn from a pool (`%YAML`, `%TAG`, `%x`, `@x` and a backtick at column 0, after a key,
an item and a flow comma, properties, markers, quoted and block scalars) and is run through yq v4.53.3, BASELINE and
SUBJECT (`succinctly` binaries, run as `yq`). A stream agrees when both refuse it, or neither does and the JSON matches.
It prints how many SUBJECT fixed, broke and left, and lists the streams it broke.
"""
import subprocess, random, sys, json
seed=int(sys.argv[1]); count=int(sys.argv[2]); base=sys.argv[3]; new=sys.argv[4]
rng=random.Random(seed)
LINES=["a: 1","- a","%YAML 1.2","%TAG ! x:","%x","%","@x","`x","a: %x","- @x","- `x","[a, %x]","{a: `x}","{%k: 1}","? %k","? a","!!str","&a","---","...","# c","a:","  b: 2","  %y: 1","  - %z","x","\"q\"","'q'","|","  txt","key: |","  line","a: >","  - c","b: [1,","  2]","%a b","- - %n","a: x","  cont","  %cont","!!map","- &a","k: !!str","\t# t","","  ","a: 'x","  y'","\"d","  e\""]
def run(b,s):
    r=subprocess.run(b.split()+["-o=json","-I0","."],input=s.encode(),capture_output=True)
    out=(r.stdout).decode(errors="replace"); err=r.stderr.decode(errors="replace")
    return ("ERR" if (r.returncode!=0 or "Error" in err) else "OK"), out
seen=set(); rows=[]
while len(rows)<count:
    n=rng.choice([2,3,3,4,5])
    s="\n".join(rng.choice(LINES) for _ in range(n))+"\n"
    if s in seen: continue
    seen.add(s); rows.append(s)
fixed=broke=same=0; bad=[]
for s in rows:
    y=run("yq",s); b=run(base+" yq",s); n=run(new+" yq",s)
    def agree(x): return x[0]==y[0] and (y[0]=="ERR" or x[1]==y[1])
    ab, an = agree(b), agree(n)
    if not ab and an: fixed+=1
    elif ab and not an: broke+=1; bad.append((s,y,b,n))
    else: same+=1
print("streams",len(rows),"fixed",fixed,"broke",broke,"unchanged",same)
for s,y,b,n in bad[:12]:
    print(repr(s)); print("   yq  ",y); print("   base",b); print("   new ",n)
