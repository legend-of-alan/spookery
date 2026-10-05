---
name: spec-test-sync
description: >
  Compare docs/spec.md to the cucumber features and Rust proofs and write
  reports/spec-test-sync.md. Use when the user runs /spec-test-sync, asks
  whether the tests still match the spec, or asks for a spec-test sync.
---

# Spec-test sync

Spawn the `spec-test-review` agent. This session does not review the files itself.

Spawn a subagent with `subagent_type` set to `spec-test-review` and this prompt:

```
Compare this spec to these tests. Return the report as markdown and do not write any file.

spec: C:\Users\alana\Local\spookery\docs\spec.md
tests: C:\Users\alana\Local\spookery\tests\features, C:\Users\alana\Local\spookery\tests\steps, C:\Users\alana\Local\spookery\tests\fixture
previous_report: C:\Users\alana\Local\spookery\reports\spec-test-sync.md
report_path: C:\Users\alana\Local\spookery\reports\spec-test-sync.md
```

If `spec-test-review` is not an available subagent type, stop and say that `C:\Users\alana\.grok\agents\spec-test-review.md` was not discovered. Do not review the files in this session.

Write the markdown the agent returns to the report path. Create `reports` if it is missing. Leave every other file unchanged, then show the report.
