# ADR 0006: nothing private in the public repo

Date: 2026-10-26. Status: accepted (owner).

**Decision.** No material, no secrets, no hostnames beyond punchable.io / auth / bugs, no other project's names.
Private operating state lives in the owner's private operations repo and `~/.punchable/`. The commit guard
(`tools/guard`) blocks violations; the fix is the content, never a bypass. Goldens and fixtures are synthetic.
