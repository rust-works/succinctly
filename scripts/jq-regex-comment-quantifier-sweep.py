#!/usr/bin/env python3
import concurrent.futures as cf
import subprocess
import sys

# #6128 (item 3): loop bodies whose repeat operator follows a comment (`(?:b(?#)*|a)+`).
#
#     scripts/jq-regex-comment-quantifier-sweep.py SUBJECT BASELINE
#
# Compares `jq` 1.7.1 with two `succinctly` binaries (the one under test and an older baseline) on
# 29,976 cases, and prints how many the subject fixed, broke and still differs on. The patterns that
# still differ all have the comment after a quantifier (`b*(?#)*`), which is kept on purpose.
# Exits 1 when the subject breaks a case the baseline answered as jq does.
JQ = '/usr/bin/jq'
NEW = sys.argv[1]
OLD = sys.argv[2]
atoms=['a','b','b*','b?','(?:b|c)','c','']
quants=['*','+','?','{2}','{1,2}','*?','{2,}']
bodies=set()
for a in atoms:
    for q in quants:
        if a=='' : continue
        # atom, comment, quantifier   (the case #6128 is about)
        bodies.add('%s(?#)%s'%(a,q)); bodies.add('%s(?#c)%s'%(a,q))
        bodies.add('%s%s'%(a,q))              # comment-free twin
alts=['a','b','c','b*','']
pats=set()
for b in bodies:
    for alt in alts:
        for order in (0,1):
            body = '%s|%s'%(b,alt) if order==0 else '%s|%s'%(alt,b)
            for loop in ('+','*','{2,}'):
                pats.add('(?:%s)%s'%(body,loop))
pats=sorted(pats)
inputs=['bab','ba','cbb','abc']
flags=['g','']
def run(exe,p,inp,fl):
    prog='"%s" | [match("%s";"%s")|[.offset,.length]]'%(inp,p.replace('\\','\\\\'),fl)
    try:
        r=subprocess.run(exe+['-nc',prog],capture_output=True,text=True,timeout=4)
        o=(r.stdout+r.stderr).strip().splitlines(); o=o[0] if o else ''
    except subprocess.TimeoutExpired: o='TIMEOUT'
    return o[:100]
def job(a):
    p,inp,fl=a
    return a,run([JQ],p,inp,fl),run([NEW,'jq'],p,inp,fl),run([OLD,'jq'],p,inp,fl)
jobs=[(p,i,f) for p in pats for i in inputs for f in flags]
print(len(jobs),'cases',flush=True)
st={'fixed':0,'broken':0,'still-differ':0,'agree-both':0,'jq-error':0}
broken=[];still=[]
with cf.ThreadPoolExecutor(8) as ex:
    for a,j,n,o in ex.map(job,jobs):
        if j.startswith('jq: error'): st['jq-error']+=1; continue
        ng=(n==j); og=(o==j)
        if ng and og: st['agree-both']+=1
        elif ng and not og: st['fixed']+=1
        elif og and not ng: st['broken']+=1; broken.append((a,j,n,o))
        else: st['still-differ']+=1; still.append((a,j,n))
print(st)
for b in broken[:15]:
    print('BROKEN', b)
import collections
c=collections.Counter(a[0] for a,_,_ in still)
print('still-differ patterns:',len(c)); 
for p,n in list(c.items())[:15]: print(n,p)

sys.exit(1 if broken else 0)
