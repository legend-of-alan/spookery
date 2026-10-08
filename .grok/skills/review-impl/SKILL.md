---
name: review-impl
description: >
  Compare an implementation to its plan and write reports/<plan>-impl.md.
  Spawns the impl-review agent. Use when the user runs /review-impl, asks
  whether the code matches the plan, or asks to review an implementation
  against a plan.
argument-hint: "[path to the plan] [base..tip]"
---

# Review an implementation

This session does not review the implementation. Spawn the `impl-review` agent.

## Arguments

The argument may name a plan, a commit range, or both. When the user names them in the request instead of the slash command, use those. A token that is an existing file is the plan path. A token that contains `..` and is not an existing file is the range.

### Plan path

- A path: use it when the file exists. When it does not, stop and say so.
- No path: when `plans/` contains one markdown file, use that file. When it contains several, list them and ask which one. When it contains none, stop and ask for a path.

### Range

- A range token: pass it as `range`.
- No range: pass `range: absent`.

## Review

If `impl-review` is not an available subagent type, stop and say that `C:\Users\alana\.grok\agents\impl-review.md` was not discovered. Do not review the implementation in this session.

Spawn a subagent with `subagent_type` set to `impl-review`, `background` set to false, and this prompt. The report path is `reports/<plan file name without .md>-impl.md` under the repo root. When the user named an extra focus in this request, add one more standing rule, in the user's words.

```
Review this implementation against this plan. Return the report as markdown and do not write any file.

plan: <absolute plan path>
spec: C:\Users\alana\Local\spookery\docs\spec.md
code: C:\Users\alana\Local\spookery
range: <base..tip or absent>
previous_report: <absolute report path>
report_path: <absolute report path>

Standing rules:
- The plan says what this change ships. Code that adds a permanent non-goal, a forever ban, or a catalog of refusal spellings is a break of this rule. Leaving later work unimplemented is not.
- The named spec is the only current spec. Check a spec sentence only when the plan names it. When the plan says a promise replaces a spec sentence, the code implements the replacement the plan names.
- A step is done only when its named outcome is visible in a file the plan names, or in a test the plan names.
- Read a file the plan does not name only when a call site there shows whether a named outcome runs.
```

Wait for the agent. Write the markdown it returns to the report path. Create `reports` if it is missing. Leave every other file unchanged, including the plan, the spec, and the code.

Show the verdict, then the findings. When the verdict is `block`, say not to commit.
