
#[cfg(test)]
mod spike_bench {
    //! Write, rollback and round costs, measured the same way on each tree. Workloads 1-4 and the
    //! rounds are PR #540's benches as PR #541 ran them; workload 5 follows PR #542's
    //! description: 100 mixed writes across three records, then one rollback over all of them.
    //! Workload 6 splits workload 5 by kind of write.
    //! Every write workload plays on one world, the docks with a `burn` modifier and a `lit`
    //! shop flag added to its canon. spike/bench_trees.py appends this to a tree and rewrites
    //! its Draft calls for trees where State is the writer.
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use crate::draft::Draft;
    use crate::intent::Intent;
    use crate::model::meter::MeterWrite;
    use crate::model::Presence;
    use crate::rules::effect::{Effect, EffectKind, EntityId};

    struct Counting;
    static ALLOCS: AtomicUsize = AtomicUsize::new(0);
    static BYTES: AtomicUsize = AtomicUsize::new(0);
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(l.size(), Ordering::Relaxed);
            System.alloc(l)
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            System.dealloc(p, l)
        }
    }
    #[global_allocator]
    static A: Counting = Counting;

    fn counts() -> (usize, usize) {
        (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
    }

    #[test]
    fn measure_writes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = crate::World::open(&root).unwrap();
        let mut canon = opened.canon().clone();
        let burn_def = crate::model::modifier::ModifierDef {
            meter: "grit".to_string(),
            max_stacks: 2,
            hidden: false,
        };
        canon.modifiers.insert("burn".to_string(), burn_def);
        let shop = canon.places.get_mut("docks_shop").unwrap();
        shop.flags.push("lit".to_string());
        let canon = std::sync::Arc::new(canon);
        let burned = crate::World::from_parts(&root, canon, opened.state().clone());
        let world = &burned;
        let nobody = Presence::default();
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let n = 20_000u32;
        let item = |delta: i64| EffectKind::Item {
            actor_id: None,
            place: Some("docks_shop".to_string()),
            id: "lamp_oil".to_string(),
            delta,
            max: None,
        };
        let meter = |actor: &str, key: &str, delta: i64| EffectKind::Meter {
            entity: EntityId::Actor(actor.to_string()),
            key: key.to_string(),
            write: MeterWrite::Offset(delta),
        };
        let apply = |draft: &mut Draft, kind: EffectKind| {
            Effect::new(intent.actor.as_ref(), &intent.id, kind)
                .apply(draft, &intent)
                .unwrap();
        };
        // 1. savepoint, one place-item write, rollback: the refused-intent path.
        let mut draft = Draft::begin(world, &nobody);
        let (a0, b0) = counts();
        let t = Instant::now();
        for _ in 0..n {
            let point = draft.savepoint();
            apply(&mut draft, item(1));
            draft.rollback(point);
        }
        let per = t.elapsed() / n;
        let (a1, b1) = counts();
        eprintln!("MEASURE w1 savepoint+place_item+rollback: {:?}, {:.1} allocs, {:.0} bytes", per, (a1 - a0) as f64 / n as f64, (b1 - b0) as f64 / n as f64);
        // 2. one actor meter write that lands, alternating up and down.
        let mut draft = Draft::begin(world, &nobody);
        let (a0, b0) = counts();
        let t = Instant::now();
        for i in 0..n {
            apply(&mut draft, meter("mira", "rest", if i % 2 == 0 { 1 } else { -1 }));
        }
        let per = t.elapsed() / n;
        let (a1, b1) = counts();
        eprintln!("MEASURE w2 landed actor_meter write: {:?}, {:.1} allocs, {:.0} bytes", per, (a1 - a0) as f64 / n as f64, (b1 - b0) as f64 / n as f64);
        // 3. a take after 40 landed writes to one actor.
        let mut draft = Draft::begin(world, &nobody);
        let t = Instant::now();
        let rounds = 500u32;
        for _ in 0..rounds {
            for i in 0..40 {
                apply(&mut draft, meter("mira", "rest", if i % 2 == 0 { 1 } else { -1 }));
            }
            let _ = draft.take_changes();
        }
        eprintln!("MEASURE w3 40 landed writes then take_changes: {:?} per batch", t.elapsed() / rounds);
        // 4. a modifier applied, then expired.
        let burn = EffectKind::Modifier {
            actor_id: "mira".to_string(),
            id: "burn".to_string(),
            add: -1,
            ticks: 3,
        };
        let mut draft = Draft::begin(world, &nobody);
        let (a0, b0) = counts();
        let t = Instant::now();
        for _ in 0..n {
            apply(&mut draft, burn.clone());
            draft.expire_modifiers(u64::MAX).unwrap();
        }
        let per = t.elapsed() / n;
        let (a1, b1) = counts();
        eprintln!("MEASURE w4 modifier applied then expired: {:?}, {:.1} allocs, {:.0} bytes", per, (a1 - a0) as f64 / n as f64, (b1 - b0) as f64 / n as f64);
        // 5. 100 mixed writes across three records (Mira's rest, the shop's oil and flag, Bram's
        // grit), then one rollback over all of them.
        let lit = |on: bool| EffectKind::Flag {
            entity: EntityId::Place("docks_shop".to_string()),
            key: "lit".to_string(),
            value: on,
        };
        let mut draft = Draft::begin(world, &nobody);
        let batches = 2_000u32;
        let (mut writes, mut rollbacks) = (Duration::ZERO, Duration::ZERO);
        let (a0, b0) = counts();
        for _ in 0..batches {
            let point = draft.savepoint();
            let t = Instant::now();
            for i in 0..25 {
                let up = i % 2 == 0;
                let by = if up { 1 } else { -1 };
                apply(&mut draft, meter("mira", "rest", by));
                apply(&mut draft, item(by));
                apply(&mut draft, lit(up));
                apply(&mut draft, meter("bram", "grit", by));
            }
            writes += t.elapsed();
            let t = Instant::now();
            draft.rollback(point);
            rollbacks += t.elapsed();
        }
        let (a1, b1) = counts();
        eprintln!("MEASURE w5 100 mixed writes: {:?} per batch; their rollback: {:?}; {:.0} allocs, {:.0} bytes per batch", writes / batches, rollbacks / batches, (a1 - a0) as f64 / batches as f64, (b1 - b0) as f64 / batches as f64);
        // 6. workload 5 split by kind: 25 writes of one kind, alternating, then one rollback.
        // Its allocation counts show which kind of write a gap in workload 5 comes from.
        let kinds: [(&str, &dyn Fn(bool) -> EffectKind); 3] = [
            ("actor meter", &|up| meter("mira", "rest", if up { 1 } else { -1 })),
            ("place item", &|up| item(if up { 1 } else { -1 })),
            ("place flag", &lit),
        ];
        for (name, kind) in kinds {
            let mut draft = Draft::begin(world, &nobody);
            let (a0, b0) = counts();
            for _ in 0..batches {
                let point = draft.savepoint();
                for i in 0..25 {
                    apply(&mut draft, kind(i % 2 == 0));
                }
                draft.rollback(point);
            }
            let (a1, b1) = counts();
            eprintln!("MEASURE w6 25 {name} writes and their rollback: {:.0} allocs, {:.0} bytes per batch", (a1 - a0) as f64 / batches as f64, (b1 - b0) as f64 / batches as f64);
        }
    }

    const WALL: &str = "2026-01-01T00:00:00Z";
    const ROUNDS: u32 = 100;

    #[test]
    fn measure_rounds() {
        let pack = crate::testing::copy_docks("round_cost_seated");
        let mut world = crate::World::resume(&pack, crate::store::lock_pack(&pack).unwrap()).unwrap();
        let seated = Presence {
            players: 2,
            actors_present: vec!["pc_rowan".to_string(), "pc_wren".to_string()],
        };
        let play = |world: &mut crate::World, i: u32| {
            let (rowan, wren) = if i % 2 == 0 {
                (("take", Some("lamp_oil")), ("wait", None))
            } else {
                (("drop", Some("lamp_oil")), ("grit", None))
            };
            let _ = world.queue_player_move(&seated, "pc_rowan", rowan.0, rowan.1);
            let _ = world.queue_player_move(&seated, "pc_wren", wren.0, wren.1);
            world.play(&seated, WALL).unwrap();
        };
        for i in 0..2 {
            play(&mut world, i);
        }
        let (a0, b0) = counts();
        let t = Instant::now();
        for i in 0..ROUNDS {
            play(&mut world, i);
        }
        let per = t.elapsed() / ROUNDS;
        let (a1, b1) = counts();
        eprintln!("MEASURE r1 seated round: {:?}, {:.0} allocs, {:.0} bytes", per, (a1 - a0) as f64 / ROUNDS as f64, (b1 - b0) as f64 / ROUNDS as f64);
        drop(world);
        let _ = std::fs::remove_dir_all(&pack);
        let pack = crate::testing::copy_docks("round_cost_idle");
        let mut world = crate::World::resume(&pack, crate::store::lock_pack(&pack).unwrap()).unwrap();
        let nobody = Presence::default();
        for _ in 0..2 {
            world.play(&nobody, WALL).unwrap();
        }
        let (a0, b0) = counts();
        let t = Instant::now();
        for _ in 0..ROUNDS {
            world.play(&nobody, WALL).unwrap();
        }
        let per = t.elapsed() / ROUNDS;
        let (a1, b1) = counts();
        eprintln!("MEASURE r2 idle round: {:?}, {:.0} allocs, {:.0} bytes", per, (a1 - a0) as f64 / ROUNDS as f64, (b1 - b0) as f64 / ROUNDS as f64);
        drop(world);
        let _ = std::fs::remove_dir_all(&pack);
    }
}
