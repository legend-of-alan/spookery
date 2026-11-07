"""Each mutant breaks one behavior State and its stores promise: (file, text, replacement) edits on
the tree spike/mutate.py exports. Each text must occur exactly once in its file when its edit is
made, in order."""

L = 'crates/engine/model/ledger.rs'
M = 'crates/engine/model/mod.rs'
T = 'crates/engine/model/tracked.rs'
W = 'crates/engine/model/write.rs'
K = 'crates/engine/canon/lint.rs'
C = 'crates/engine/store/commit.rs'

# Not mutants: lets a mutant tell the undo of an add (an entry taken off) or of a removal (an
# entry put back whole) from other undos, and undo one store while skipping some of them. The
# forward writes and every other undo stay as they are.
UNDO_KINDS = [
    (T, """    fn apply(self, value: &mut T) -> Option<Self>;
}""", """    fn apply(self, value: &mut T) -> Option<Self>;
    fn takes_off(&self) -> bool {
        false
    }
    fn puts_back(&self) -> bool {
        false
    }
}"""),
    (T, """impl<V: PartialEq> Edit<BTreeMap<String, V>> for Entry<V> {
""", """impl<V: PartialEq> Edit<BTreeMap<String, V>> for Entry<V> {
    fn takes_off(&self) -> bool {
        self.value.is_none()
    }
    fn puts_back(&self) -> bool {
        self.value.is_some()
    }
"""),
    (T, """impl<V: PartialEq, E: Edit<V>> Edit<BTreeMap<String, V>> for MapEdit<V, E> {
""", """impl<V: PartialEq, E: Edit<V>> Edit<BTreeMap<String, V>> for MapEdit<V, E> {
    fn takes_off(&self) -> bool {
        matches!(self, MapEdit::Entry(entry) if entry.takes_off())
    }
    fn puts_back(&self) -> bool {
        matches!(self, MapEdit::Entry(entry) if entry.puts_back())
    }
"""),
    (T, """impl<V: PartialEq, E: Edit<V>> Edit<EntityMaps<V>> for EntityEdit<V, E> {
""", """impl<V: PartialEq, E: Edit<V>> Edit<EntityMaps<V>> for EntityEdit<V, E> {
    fn takes_off(&self) -> bool {
        self.edit.takes_off()
    }
    fn puts_back(&self) -> bool {
        self.edit.puts_back()
    }
"""),
    (T, """impl<T, U: Edit<T>> Store<T, U> {
""", """impl<T, U: Edit<T>> Store<T, U> {
    pub(super) fn undo_to_but(&mut self, seq: u64, skip: impl Fn(&U) -> bool) {
        while self.log.last().is_some_and(|(at, _)| *at >= seq) {
            let (_, undo) = self.log.pop().expect("an undo");
            if !skip(&undo) {
                undo.apply(&mut self.value);
            }
        }
    }
"""),
]

UNDO_ONE = """            let redo = undo.apply(&mut self.value);
            debug_assert!(redo.is_some(), "an undo changed nothing");"""

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
    # Insertion rolls back: in any store, undoing an added entry leaves it on.
    'added_stays': UNDO_KINDS + [(T, UNDO_ONE, """            if !undo.takes_off() {
                undo.apply(&mut self.value);
            }""")],
    # An item put on records no undo.
    'item_added_untracked': [(T, """impl<V: PartialEq, E: Edit<V>> PartsMut<'_, V, E> {
""", """impl<V: PartialEq, E: Edit<V>> PartsMut<'_, V, E> {
    pub(super) fn apply_untracked(self, edit: MapEdit<V, E>) {
        match self {
            PartsMut::Stored(store, id) => {
                if store.apply(EntityEdit { id: id.to_string(), edit }, || 0) {
                    store.log.pop();
                }
            }
            PartsMut::World(store) => {
                if store.apply(edit, || 0) {
                    store.log.pop();
                }
            }
        }
    }
"""), (W, "        items.apply(edit, || ledger.issue_seq());", """        if held == 0 {
            items.apply_untracked(edit);
        } else {
            items.apply(edit, || ledger.issue_seq());
        }""")],
    # Removal rolls back, the removed entry whole: in any store, undoing a removal does not put
    # the entry back.
    'removed_entry_lost': UNDO_KINDS + [(T, UNDO_ONE, """            if !undo.puts_back() {
                undo.apply(&mut self.value);
            }""")],
    # The same, one kind of component at a time.
    'removed_modifier_lost': UNDO_KINDS + [(M, "        modifiers.undo_to(seq);", "        modifiers.undo_to_but(seq, |undo| undo.puts_back());")],
    'removed_flag_lost': UNDO_KINDS + [
        (M, """        modifiers.undo_to(seq);
        flags.undo_to(seq);""", """        modifiers.undo_to(seq);
        flags.undo_to_but(seq, |undo| undo.puts_back());"""),
        (M, """        flags.undo_to(seq);
        open_exits.undo_to(seq);""", """        flags.undo_to_but(seq, |undo| undo.puts_back());
        open_exits.undo_to(seq);"""),
        (M, """        let StoryComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to(seq);""", """        let StoryComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to_but(seq, |undo| undo.puts_back());"""),
        (M, """        let WorldComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to(seq);""", """        let WorldComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to_but(seq, |undo| undo.puts_back());"""),
    ],
    'removed_item_lost': UNDO_KINDS + [
        (M, """        records.undo_to(seq);
        items.undo_to(seq);""", """        records.undo_to(seq);
        items.undo_to_but(seq, |undo| undo.puts_back());"""),
        (M, """        items.undo_to(seq);
        meters.undo_to(seq);
        flags.undo_to(seq);""", """        items.undo_to_but(seq, |undo| undo.puts_back());
        meters.undo_to(seq);
        flags.undo_to(seq);"""),
    ],
    # A door's undo is lost: the doors' store is left out of the places' rollback.
    'door_undo_lost': [(M, "        open_exits.undo_to(seq);\n", "")],
    # Every store rolls back: each one left out of its kind's rollback.
    'records_not_undone': [(M, "        records.undo_to(seq);\n", "")],
    'story_meters_not_undone': [(M, """        let StoryComponents { meters, flags } = self;
        meters.undo_to(seq);
""", """        let StoryComponents { meters, flags } = self;
""")],
    'world_meters_not_undone': [(M, """        let WorldComponents { meters, flags } = self;
        meters.undo_to(seq);
""", """        let WorldComponents { meters, flags } = self;
""")],
    'notes_not_undone': [(M, "        self.notes.undo_to(seq);\n", "")],
    # A field no key answers for still rolls back.
    'visited_undo_wrong': [(M, "            ActorEdit::Visited(edit) => ActorEdit::Visited(edit.apply(&mut actor.visited)?),", """            ActorEdit::Visited(edit) => {
                let id = edit.id.clone();
                edit.clone().apply(&mut actor.visited)?;
                ActorEdit::Visited(Entry { id, value: edit.value })
            }""")],
    # The seq boundary: a rollback undoes the first change after its savepoint too.
    'seq_boundary': [(T, "|(at, _)| *at >= seq)", "|(at, _)| *at > seq)")],
    # Order within a store: a rollback undoes its changes oldest first.
    'undo_oldest_first': [(T, """        while self.log.last().is_some_and(|(at, _)| *at >= seq) {
            let (_, undo) = self.log.pop().expect("an undo");
            // The change it undoes was the newest one, so the value is as that change left it.
""" + UNDO_ONE + """
        }""", """        let from = (self.log.iter().position(|(at, _)| *at >= seq)).unwrap_or(self.log.len());
        for (_, undo) in self.log.drain(from..).collect::<Vec<_>>() {
            undo.apply(&mut self.value);
        }""")],
    # G2: an entity's map is dropped when a change takes off its last entry...
    'map_not_dropped': [(T, """                if parts.is_empty() {
                    maps.0.remove(&id);
                }""", "")],
    # ... and put back when a change puts on its first.
    'map_not_recreated': [(T, """                if !parts.is_empty() {
                    maps.0.insert(id.clone(), parts);
                }""", "")],
    # Only an accept forgets undo: here a rollback forgets every undo an enclosing savepoint
    # still needs.
    'rebase_under_enclosing': [(L, """        self.undo_to(point.seq);
""", """        self.undo_to(point.seq);
        self.clear_log();
""")],
    # The outer commit accepts.
    'commit_skips_accept': [(C, """    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;
    state.accept();""", """    let mut log_path = append_to_bundle(&world.root, &payload, &write)?;""")],
    # An accept forgets every undo.
    'accept_keeps_logs': [(L, """        self.clear_log();
        self.ledger = Ledger::default();""", """        self.ledger = Ledger::default();""")],
    # A rollback leaves the seq where it was. Expected equivalent: see the step's report.
    'rollback_restores_seq': [(L, """        self.undo_to(point.seq);
        let ledger = &mut self.ledger;""", """        self.undo_to(point.seq);
        let ledger = &mut self.ledger;
        ledger.seq = point.seq;""")],
    # Seats are the round's: count_act reads who is seated.
    'seats_ignored': [(W, "        let (counter, count) = if self.presence().players == 0 {", "        let (counter, count) = if true {")],
    # A component on an actor must be on one the save holds a record of (C5).
    'actor_ids_unchecked': [(K, "        if !actors.records().contains_key(&id) {", "        if false {")],
}
