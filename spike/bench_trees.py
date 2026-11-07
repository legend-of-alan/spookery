"""Run spike/bench.rs on several trees of this repository, interleaved, in release.

    python spike/bench_trees.py <scratch dir> <name>=<rev> [<name>=<rev> ...] [--runs N]

Each tree is a `git archive` export of <rev> with spike/bench.rs appended to the file that
holds its round writer: crates/engine/draft.rs where Draft exists, else crates/engine/model/
write.rs, with the bench's Draft calls rewritten to the State API by the fixed substitutions
below. All trees share one target dir. Prints one line per MEASURE line, prefixed with the
tree and run, and writes the same to <scratch>/results.txt.
"""
import os
import shutil
import subprocess
import sys

STATE_API = [
    ("use crate::draft::Draft;\n", ""),
    ("let apply = |draft: &mut Draft, kind: EffectKind| {", "let apply = |draft: &mut crate::model::State, kind: EffectKind| {"),
    (".apply(draft, &intent)", ".apply(world.canon(), draft, &intent)"),
    ("Draft::begin(world, &nobody)", "world.begin_round(&nobody)"),
    ("draft.expire_modifiers(u64::MAX)", "draft.expire_modifiers(world.canon(), u64::MAX)"),
]

args = [a for a in sys.argv[1:] if not a.startswith('--')]
runs = int(sys.argv[sys.argv.index('--runs') + 1]) if '--runs' in sys.argv else 3
args = [a for a in args if not a.isdigit()]
scratch, trees = os.path.abspath(args[0]), [a.split('=', 1) for a in args[1:]]
repo = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True,
                      check=True).stdout.strip()
bench = open(os.path.join(repo, 'spike', 'bench.rs'), encoding='utf-8').read()
env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(scratch, 'target'))
for name, rev in trees:
    tree = os.path.join(scratch, name)
    shutil.rmtree(tree, ignore_errors=True)
    os.makedirs(tree)
    archive = subprocess.run(['git', '-C', repo, 'archive', rev], capture_output=True, check=True).stdout
    subprocess.run(['tar', '-x', '-C', tree], input=archive, check=True)
    draft = os.path.join(tree, 'crates', 'engine', 'draft.rs')
    if os.path.exists(draft):
        host, text = draft, bench
    else:
        host, text = os.path.join(tree, 'crates', 'engine', 'model', 'write.rs'), bench
        for old, new in STATE_API:
            assert old in text, old
            text = text.replace(old, new)
    with open(host, 'a', encoding='utf-8', newline='\n') as out:
        out.write(text)
    build = subprocess.run(['cargo', 'test', '--release', '-p', 'spookery-engine', '--lib', '--no-run'],
                           cwd=tree, env=env, capture_output=True, text=True)
    if build.returncode:
        sys.exit(f'{name} does not build:\n{build.stderr[-4000:]}')
with open(os.path.join(scratch, 'results.txt'), 'w', encoding='utf-8') as results:
    for run in range(runs):
        for name, _ in trees:
            out = subprocess.run(['cargo', 'test', '--release', '-p', 'spookery-engine', '--lib',
                                  'spike_bench', '--', '--nocapture', '--test-threads=1'],
                                 cwd=os.path.join(scratch, name), env=env, capture_output=True,
                                 text=True, encoding='utf-8', errors='replace')
            for line in (out.stdout + out.stderr).splitlines():
                if 'MEASURE' in line:
                    row = f'{name} run{run} {line.strip()}'
                    print(row, flush=True)
                    results.write(row + '\n')
