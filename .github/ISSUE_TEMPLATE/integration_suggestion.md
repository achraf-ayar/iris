---
name: Suggest a signal or integration
about: Propose a new session signal, panel, column, or integration for iris to surface
labels: ["integration", "needs-triage"]
---

## Signal / integration name

<!-- Short, scannable. Example: "Stuck-in-loop detector", "Per-session git branch column" -->

## What would it surface?

<!-- One sentence describing the information or capability you want in the dashboard. -->

## Why does it matter?

<!-- What supervision pain does it solve? A real example from your own multi-session workflow is the best signal. -->

## Where would it live?

- [ ] Sessions list (new column / glyph)
- [ ] Detail / activity feed
- [ ] A new panel
- [ ] The approval flow
- [ ] The `ls` table output

## Where would the data come from?

<!-- Transcript fields iris already reads? A new file under ~/.claude/? An external command (git, gh)? -->

## Does it need a network call?

<!-- Y / N. iris is local-first — the only outbound calls today are opt-in AI summaries and risk reads. -->
