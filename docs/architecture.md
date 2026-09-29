# Spec index

This path is not the spec. It says which file is.

| File | Status |
|---|---|
| `docs/architecture_v0.2.md` | Frozen normative spec for v0.2. Gates A–C are proved. |
| `docs/architecture_v0.1.md` | Frozen normative spec for the v0.1 sitting. Gates A–E are proved. Tag `v0.1` is `b1d4fd1`. |
| `docs/architecture_v0.md` | Frozen normative spec for v0. It still holds where a later file does not name a change. Do not edit it to describe a later sitting. |
| `docs/product_spec.md` | Non-normative. |

Where v0.2 names a change, v0.2 wins. Where it does not, v0.1 wins for the change it names, then v0. A rule written in none of them is out of scope. Player lines on this tag are `say`, `talk`, `wait`, `attempt <check> <target>`, and `travel <place>`. On the v0.1 tag they are `say`, `talk`, `wait`, and `attempt <check> <target>`.

`docs/architecture_v0.2.md` §2, where it says the proof files stay unrewritten and that there is no shared test harness, is the freeze record. The proof files now live under `tests/conformance/v0/`, `tests/conformance/v01/`, and `tests/conformance/v02/`, and the paths named in the frozen specs stay that record.
