"""Show that round code outside `model` cannot change the save past `State`'s writers.

    python spike/bypass_check.py <scratch dir> [<rev>]

Exports <rev> (default HEAD), appends each group of attempts below to crates/engine/rules/
travel.rs (a round module outside `model`), runs `cargo check`, and prints the errors those
lines get. Privacy errors stop rustc before borrowck, so the groups are checked one at a time,
and the writes through a getter, which only borrowck refuses, are a group of their own.
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
fn bypass(state: &mut State, entity: crate::rules::effect::EntityId) {
    state.actors().meters().apply(unreachable!(), || 0);
    state.places().flags().undo_to(0);
    state.actors().items().clear_log();
    state.world().meters().apply(unreachable!(), || 0);
    state.meters_of_mut(&entity, "grit");
    state.notes.frontier_mut();
    state.notes.apply(unreachable!(), || 0);
    state.notes.undo_to(0);
    state.actors().undo_to(0);
    state.undo_to(0);
    state.clear_log();
    crate::model::tracked::Store::<Vec<crate::model::Note>, crate::model::NoteEdit>::default().apply(unreachable!(), || 0);
    crate::model::tracked::Store::<Vec<crate::model::Note>, crate::model::NoteEdit>::default().undo_to(0);
    crate::model::tracked::Store::<Vec<crate::model::Note>, crate::model::NoteEdit>::default().clear_log();
    crate::model::ledger::Ledger::default().issue_seq();
    let ledger = &mut state.ledger;
    state.normalize();
    state.world_time = 0;
}
''',
    'record tables': '''
#[allow(dead_code, unused, unreachable_code)]
fn bypass(state: &mut State, actor: crate::model::ActorLive) {
    state.actors().records().apply(unreachable!(), || 0);
    state.actors.records.undo_to(0);
    state.actors().records.clear_log();
    state.places().open_exits.remove("docks_shop");
    state.places().open_exits().undo_to(0);
    state.world_time = 0;
}
''',
    'whole-struct and whole-store replacement': '''
#[allow(dead_code, unused)]
fn bypass(state: &mut State) {
    state.actors = Default::default();
    std::mem::take(&mut state.places);
    std::mem::swap(&mut state.world, &mut Default::default());
    std::mem::replace(&mut state.stories, Default::default());
    state.notes = Default::default();
    std::mem::take(&mut state.actors().meters);
    std::mem::take(state.actors_mut());
    std::mem::swap(state.places().open_exits(), &mut Default::default());
    state.world_time = 0;
}
''',
    'writes through a getter': '''
#[allow(dead_code, unused)]
fn bypass(state: &mut State, actor: crate::model::ActorLive) {
    *state.actors().meters() = Default::default();
    *state.world() = Default::default();
    *state.places().open_exits() = Default::default();
    state.actors().records().insert(String::new(), actor);
    state.actors().records().remove("mira");
    state.world().flags().insert(String::new(), ());
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
