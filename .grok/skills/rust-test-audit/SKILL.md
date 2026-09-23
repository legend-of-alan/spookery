---
name: rust-test-audit
description: >
  Light read-only pass over a Rust diff for tests that cannot fail,
  new branches with no test, test names that no longer match the code,
  and leftover ignore or todo tests. Use when the user runs
  /rust-test-audit, asks to audit or tighten Rust tests, or asks which
  tests went stale after a change. Reports a handful of notes and does
  not edit code.
---

# Rust test audit

Read-only. Report at most 5 notes. Each note is a path, a line, and one sentence. If the range has no Rust changes, say so and stop. If none of the four checks hit, say so in one sentence.

Leave the tree unchanged. Skip planning docs, coverage, `cargo mutants`, `llvm-cov`, and the test suite. A check is a read of the diff.

## Scope

Use the git range the user names. Otherwise use uncommitted `*.rs` changes plus:

```bash
git diff --name-only $(git merge-base HEAD origin/main)...HEAD -- '*.rs'
```

If `origin/main` is missing, use `main`.

When that list is longer than about ten files, read added or changed `match` arms, `if` conditions, and `Err` returns, plus tests that name those functions. Stop at 5 notes.

Look for tests in the same file's `#[cfg(test)]` module and in a sibling test file whose name matches the changed module. Do not search the rest of the repo.

## Checks

1. **Cannot fail.** The only check is `assert!(true)`, `assert_eq!(x, x)`, `is_ok()`, `is_some()`, or `len() > 0`; or `#[should_panic]` has no `expected`; or the expected value is produced by calling the function under test.
2. **New arm, no test.** A new or changed branch, `match` arm, or `Err` path has no test that supplies the input which selects it.
3. **Name drifted.** The test name states a behavior, and the assertion never looks at the part of that function the diff changed.
4. **Left behind.** `#[ignore]`, `todo!()`, or `unimplemented!()` sits in a test whose code is now present in the diff.

For (2) and (3), ask whether the test would still pass if that branch were deleted or the condition flipped.

## Notes

```
path:line — which check failed, in one sentence
```

A hand run against the branch diff is the intended use. A scheduled run must pass that same range and must not widen it.
