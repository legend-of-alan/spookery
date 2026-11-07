"""Show that round code outside `model` cannot change the save past the ledger.

    python spike/bypass_check.py <scratch dir> [<rev>]

Exports <rev> (default HEAD), appends each group of attempts below to crates/engine/rules/
travel.rs (a round module outside `model`), runs `cargo check`, and prints the errors those
lines get. Privacy errors stop rustc before borrowck, so the groups are checked one at a time.
Every line but the last in each group must be refused; the clock field is the documented
exception.
"""
import os
import shutil
import subprocess
import sys

GROUPS = {
    'private mutators': '''
#[allow(dead_code, unused, unreachable_code)]
fn bypass(state: &mut State, holder: Holder) {
    state.meters.take(&holder, "grit");
    state.flags.put(holder.clone(), String::new(), unreachable!());
    state.items.tracked_mut(&holder, "lamp_oil");
    state.notes.frontier_mut();
    state.notes.apply(unreachable!());
    let ledger = &mut state.ledger;
    state.normalize();
    state.world_time = 0;
}
''',
    'read-only tables': '''
#[allow(dead_code, unused)]
fn bypass(state: &mut State, actor: crate::model::ActorLive) {
    state.actors.insert(String::new(), actor.into());
    state.places.remove("docks_shop");
    state.actors.get_mut("mira");
    state.world_time = 0;
}
''',
}

scratch = os.path.abspath(sys.argv[1])
rev = sys.argv[2] if len(sys.argv) > 2 else 'HEAD'
repo = subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True,
                      check=True).stdout.strip()
env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(scratch, 'target'))
for name, attempts in GROUPS.items():
    tree = os.path.join(scratch, 'tree')
    shutil.rmtree(tree, ignore_errors=True)
    os.makedirs(tree)
    archive = subprocess.run(['git', '-C', repo, 'archive', rev], capture_output=True, check=True).stdout
    subprocess.run(['tar', '-x', '-C', tree], input=archive, check=True)
    with open(os.path.join(tree, 'crates', 'engine', 'rules', 'travel.rs'), 'a', encoding='utf-8',
              newline='\n') as out:
        out.write(attempts)
    check = subprocess.run(['cargo', 'check', '-p', 'spookery-engine', '--message-format', 'short'],
                           cwd=tree, env=env, capture_output=True, text=True, encoding='utf-8',
                           errors='replace')
    print(f'== {name}')
    for line in check.stderr.splitlines():
        if 'travel.rs' in line and 'error' in line:
            print('  ', line.split('travel.rs', 1)[1].strip())
