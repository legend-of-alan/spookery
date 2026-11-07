"""Count engine lines: python count.py <tree root>.

Production: lines in crates/engine/**/*.rs that are not blank and not `//` comments, leaving
out tests/, round/prop/ and #[cfg(test)] items (the attribute through the end of the item it
covers). Test: the lines left out, plus every file under crates/*/tests (same blank and comment
rule). Per-file production counts go to stdout after the totals.
"""
import os
import sys

root = sys.argv[1]


def counted(line):
    s = line.strip()
    return bool(s) and not s.startswith('//')


def split_cfg_test(lines):
    """(production lines, test lines) of one file, by brace depth after #[cfg(test)]."""
    prod, test = [], []
    i = 0
    while i < len(lines):
        if lines[i].strip() == '#[cfg(test)]':
            # The item runs to the line where its braces close, or to a `;` at depth 0.
            depth = 0
            opened = False
            j = i
            while j < len(lines):
                code = lines[j].split('//')[0]
                depth += code.count('{') - code.count('}')
                opened = opened or '{' in code
                test.append(lines[j])
                if (opened and depth <= 0) or (not opened and code.rstrip().endswith(';')):
                    break
                j += 1
            i = j + 1
            continue
        prod.append(lines[i])
        i += 1
    return prod, test


eng = os.path.join(root, 'crates', 'engine')
prod_total, test_total, per_file = 0, 0, {}
for dirpath, _, files in os.walk(eng):
    rel_dir = os.path.relpath(dirpath, eng).replace('\\', '/')
    for name in files:
        if not name.endswith('.rs'):
            continue
        path = os.path.join(dirpath, name)
        lines = open(path, 'rb').read().decode().split('\n')
        rel = (rel_dir + '/' + name).lstrip('./')
        if rel_dir == 'tests' or rel_dir.startswith('tests/') or rel_dir.startswith('round/prop'):
            test_total += sum(map(counted, lines))
            continue
        prod, test = split_cfg_test(lines)
        n = sum(map(counted, prod))
        prod_total += n
        test_total += sum(map(counted, test))
        per_file[rel] = n
for crate in os.listdir(os.path.join(root, 'crates')):
    if crate == 'engine':
        continue
    tdir = os.path.join(root, 'crates', crate, 'tests')
    for dirpath, _, files in os.walk(tdir):
        for name in files:
            if name.endswith('.rs'):
                lines = open(os.path.join(dirpath, name), 'rb').read().decode().split('\n')
                test_total += sum(map(counted, lines))
print('engine_production', prod_total)
print('tests', test_total)
for rel in sorted(per_file):
    print(per_file[rel], rel)
