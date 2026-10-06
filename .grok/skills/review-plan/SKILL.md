---
name: review-plan
description: >
  Adversarial architecture review of a design or implementation plan.
  Spawns the architecture-review agent and writes the report under reports/.
  Use when the user runs /review-plan, asks to review a plan, or asks for
  an adversarial review of a new design.
argument-hint: "[path to the plan]"
---

# Review a plan

This session does not review the plan. Spawn the `architecture-review` agent.

## Plan path

The argument is the plan file.

- A path: use it when the file exists. When it does not, stop and say so.
- No path: when `plans/` contains one markdown file, use that file. When it contains several, list them and ask which one. When it contains none, stop and ask for a path.

## Review

If `architecture-review` is not an available subagent type, stop and say that `C:\Users\alana\.grok\agents\architecture-review.md` was not discovered. Do not review the plan in this session.

Spawn a subagent with `subagent_type` set to `architecture-review`, `background` set to false, and this prompt. The report path is `reports/<plan file name without .md>-review.md` under the repo root. When the user named an extra focus in this request, add one more standing rule, in the user's words.

```
Review this plan. Return the report as markdown and do not write any file.

plan: <absolute plan path>
spec: C:\Users\alana\Local\spookery\docs\spec.md
code: C:\Users\alana\Local\spookery
report_path: <absolute report path>

Standing rules:
- The plan says what this change ships. A permanent non-goal, a forever ban, or a catalog of refusal spellings is a break of this rule. Leaving later work unspecified is not.
- The named spec is the only current spec. A second architecture document that would stay in force beside it is a break of this rule. The plan file itself is the proposal under review.
- When the plan changes a promise, it names the spec sentence this change replaces.
- The plan has an ordered implementation sequence. Each step names what it touches and an outcome a reader could check.
```

Wait for the agent. Write the markdown it returns to the report path. Create `reports` if it is missing. Leave every other file unchanged, including the plan and the spec.

Show the verdict, then the findings.
