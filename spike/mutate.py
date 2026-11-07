"""Run each mutant in spike/mutants.py on an export of a revision and list what catches it.

    python spike/mutate.py <scratch dir> [<rev>] [mutant ...]

Exports <rev> (default HEAD) with `git archive`, applies one mutant at a time, runs
`cargo test --workspace --no-fail-fast` there with a shared target dir, and appends to
<scratch>/report.txt the scenarios and tests that failed. "NOTHING CAUGHT IT" marks a survivor.
"""
import os
import re
import shutil
import subprocess
import sys

scratch = os.path.abspath(sys.argv[1])
rev = sys.argv[2] if len(sys.argv) > 2 else 'HEAD'
repo = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True,
                      check=True).stdout.strip()
scope = {}
exec(open(os.path.join(repo, 'spike', 'mutants.py'), encoding='utf-8').read(), scope)
mutants = scope['MUTANTS']
names = sys.argv[3:] or list(mutants)
tree = os.path.join(scratch, 'tree')
env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(scratch, 'target'))
os.makedirs(scratch, exist_ok=True)
report = open(os.path.join(scratch, 'report.txt'), 'a', encoding='utf-8')
for name in names:
    shutil.rmtree(tree, ignore_errors=True)
    os.makedirs(tree)
    archive = subprocess.run(['git', '-C', repo, 'archive', rev], capture_output=True, check=True).stdout
    subprocess.run(['tar', '-x', '-C', tree], input=archive, check=True)
    for path, old, new in mutants[name]:
        file = os.path.join(tree, path)
        text = open(file, 'rb').read().decode()
        assert text.count(old) == 1, (name, path, old[:60])
        open(file, 'wb').write(text.replace(old, new).encode())
    run = subprocess.run(['cargo', 'test', '--workspace', '--no-fail-fast'], cwd=tree, env=env,
                         capture_output=True)
    log = (run.stdout + run.stderr).decode('utf-8', 'replace')
    open(os.path.join(scratch, name + '.log'), 'w', encoding='utf-8').write(log)
    caught = []
    for line in log.split('\n'):
        # A failed step names its own line; the scenario is the nearest title above it, read
        # from the feature file, since concurrent scenarios interleave their output.
        found = re.search(r'Defined: \??\\?(.*\.feature):(\d+):', line)
        if found:
            feature = found.group(1).replace('\\', '/')
            lines = open(feature, encoding='utf-8').read().split('\n')[: int(found.group(2))]
            titles = [l.strip() for l in lines if l.strip().startswith('Scenario')]
            title = 'scenario: ' + (titles[-1].split(':', 1)[1].strip() if titles else feature)
            if title not in caught:
                caught.append(title)
        found = re.match(r'test (\S+) \.\.\. FAILED', line)
        if found:
            caught.append('test: ' + found.group(1))
        if line.startswith('error[') or line.startswith('error: could not compile'):
            caught.append('COMPILE: ' + line)
    report.write(f'== {name} (exit {run.returncode})\n')
    for item in caught or ['NOTHING CAUGHT IT']:
        report.write('   ' + item + '\n')
    report.flush()
    print(name, len(caught), flush=True)
