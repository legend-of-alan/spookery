# Spec index

This path is not the spec. It says which file is.

| File | Status |
|---|---|
| `docs/architecture_v0.2.md` | Draft for the v0.2 minor. Patch `v0.2.0` is proved. Tag `v0.2.0` is `913eeb6`. The bare tag `v0.2` is cut when that file's demo is true. |
| `docs/architecture_v0.1.md` | Frozen normative spec for the v0.1 sitting. Gates A–E are proved. Tag `v0.1` is `b1d4fd1`. |
| `docs/architecture_v0.md` | Frozen normative spec for v0. It still holds where a later file does not name a change. Do not edit it to describe a later sitting. |
| `docs/product_spec.md` | Non-normative. |

Where v0.2 names a change, v0.2 wins. Where it does not, v0.1 wins for the change it names, then v0. A rule written in none of them is out of scope. On tag `v0.2.0`, player lines are `say`, `talk`, `wait`, `attempt <check> <target>`, and `travel <place>`. On the v0.1 tag they are `say`, `talk`, `wait`, and `attempt <check> <target>`. The open minor adds a player verb only when a patch in `docs/architecture_v0.2.md` says so.

`docs/architecture_v0.md` and `docs/architecture_v0.1.md` stay frozen. The v0.2.0 proofs are `tests/conformance/v02/gates.rs` functions `v02_p01` through `v02_p05`, compiled through `tests/sitting.rs`.
