#!/usr/bin/env python3
"""Differential sweep for #6413: random streams of own-line node properties, keys and items that leave their value
to the next line, block scalars, and a `- ` in a flow collection.

    scripts/yq-property-line-sweep.py SEED COUNT BASELINE SUBJECT

Each stream is two to five lines drawn from a pool and is run through yq v4.53.3, BASELINE and SUBJECT (`succinctly`
binaries, run as `yq`). A stream agrees when both refuse it, or neither does and the JSON matches. It prints how many
SUBJECT fixed, broke and left, and lists the streams it broke. Same harness as `yq-directive-in-document-sweep.py`.
"""
import subprocess, random, sys, json
ver=subprocess.run(["yq","--version"],capture_output=True,text=True).stdout
assert "v4.53.3" in ver, "the oracle is yq v4.53.3, not "+ver.strip()
seed=int(sys.argv[1]); count=int(sys.argv[2]); base=sys.argv[3]; new=sys.argv[4]
rng=random.Random(seed)
LINES=["[a, - x]","{a: - x}","[- y]","a: 1","b:","c: !!str","d: &x","- a","-","- !!str","- &y","!!map","&z","!!str &w","  !!str","  &v","  x","  b: 2","  - c","k: |","  txt","k: >","- k: |","x","\"q\"","[a, b]","? k",": v","# c","","!!seq","  !!map","  &a b: 1","&a c: 2","- &a","  - d","\tx","...","---","e: !!int 3","f: [1,","  2]"]
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
