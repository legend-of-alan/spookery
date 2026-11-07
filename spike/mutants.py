"""Each mutant breaks one behavior the State ledger promises: (file, text, replacement) edits on
the tree spike/mutate.py exports. Each text must occur exactly once in its file."""

L = 'crates/engine/model/ledger.rs'
M = 'crates/engine/model/mod.rs'
W = 'crates/engine/model/write.rs'
K = 'crates/engine/canon/lint.rs'
C = 'crates/engine/store/commit.rs'

MUTANTS = {
    # A key moved and moved back since the last take is no change.
    'aba_filter': [(L, ".filter(|change| change.before != change.after)", ".filter(|_| true)")],
    # Refusal precedence: the meter's key, then its range, then the modifier's own key.
    'own_key_first': [(W, """        if let (Some(meter), Some(before)) = (meter, meter_before) {""", """        if moved {
            let own = StateKey::ActorModifier(actor_id.to_string(), id.to_string());
            self.ledger.count_key(&own)?;
        }
        if let (Some(meter), Some(before)) = (meter, meter_before) {""")],
    # Main's budget: a modifier spends its meter's key too.
    'no_meter_key': [(W, """            if stacks_moved {
                self.ledger.count_key(&key)?;
            }""", "")],
    'no_overflow_check': [(W, """            if puts {
                after?;
            }""", "")],
    # Effective-value notification for the meter a modifier adds to.
    'no_derived_note': [(W, "                self.ledger.keep(Change { key, before, after });", "                let _ = (key, before, after);")],
    # Insertion rolls back.
    'added_stays': [(L, "                Touch::Added(at) => self.take_off(&at),", "                Touch::Added(_) => {}")],
    'item_added_untracked': [(W, """            items.put(id.to_string(), Tracked::new(count));
            ledger.added(at());""", """            items.put(id.to_string(), Tracked::new(count));""")],
    # Removal rolls back, the removed component whole.
    'removed_modifier_lost': [(M, "            Removed::Modifier(actor, id, part) => self.actors.modifiers.put(&actor, id, part),", "            Removed::Modifier(..) => {}")],
    'removed_flag_lost': [(M, """            Removed::Flag(entity, key) => {
                self.flags_of_mut(&entity, &key)
                    .0
                    .put(key.clone(), Tracked::default());
            }""", "            Removed::Flag(..) => {}")],
    'removed_item_lost': [(M, """            Removed::Item(entity, id, part) => {
                if let Some((mut items, ..)) = self.items_of_mut(&entity, &id) {
                    items.put(id.clone(), part);
                }
            }""", "            Removed::Item(..) => {}")],
    # A field no key answers for still rolls back.
    'visited_undo_wrong': [(M, "            ActorEdit::Visited(edit) => ActorEdit::Visited(edit.apply(&mut actor.visited)?),", """            ActorEdit::Visited(edit) => {
                let id = edit.id.clone();
                edit.clone().apply(&mut actor.visited)?;
                ActorEdit::Visited(Entry { id, value: edit.value })
            }""")],
    # Rebase only when no enclosing rollback needs the history: here a rollback rebases every
    # component still touched, which an enclosing savepoint needs.
    'rebase_under_enclosing': [(L, """        let ledger = &mut self.ledger;
        for key in ledger.key_order.drain(point.keys..) {""", """        let touched: Vec<ComponentId> = (self.ledger.touched.iter())
            .filter_map(|touch| match touch {
                Touch::Edited(at, _) => Some(at.clone()),
                _ => None,
            })
            .collect();
        for at in touched {
            if let Some(part) = self.component_mut(&at) {
                part.rebase(0);
            }
        }
        let ledger = &mut self.ledger;
        for key in ledger.key_order.drain(point.keys..) {""")],
    # The outer commit rebases.
    'commit_skips_accept': [(C, """    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;
    state.accept();""", """    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;""")],
    # Seats are the round's: count_act reads who is seated.
    'seats_ignored': [(W, "        let (counter, count) = if self.presence().players == 0 {", "        let (counter, count) = if true {")],
    # A component on an actor must be on one the save holds a record of (C5).
    'actor_ids_unchecked': [(K, "        if !actors.records().contains_key(&id) {", "        if false {")],
}
