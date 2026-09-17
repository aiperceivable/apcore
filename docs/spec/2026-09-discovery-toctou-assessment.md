---
description: "Threat-model assessment of the time-of-check/time-of-use window in extension discovery, deferred out of D-94 and D-127 rather than decided by them."
---

# Discovery TOCTOU — deferred security assessment

**Status: open. No decision, no SDK change.** This exists because D-94 and D-127
both say "TOCTOU is out of scope, filed separately", and a pointer that resolves
to nothing is worse than no pointer.

## What the current design guarantees

Two decisions govern symlinks during extension discovery:

- **D-94** — the containment check runs **before** the directory/file split, so a
  symlink whose target escapes the canonical extensions root is skipped whether
  it resolves to a directory or a file.
- **D-127** — `follow_symlinks: true` records the real target **once**; file
  identity, module ID and visited-directory tracking are all keyed on the
  canonical real path.

Together they guarantee this: **at the moment the scanner looked, every path it
recorded resolved inside the root, and it recorded each real file once.**

## What they do not guarantee

They say nothing about the interval between that look and the load. The scanner
resolves a path, records it, finishes the scan, and the loader opens the file
afterwards. A party who can write to the extensions root can replace a symlink in
between, and the loader then opens a target that was never checked.

That is the classic time-of-check/time-of-use window, and it is not closed by
checking harder. `realpath` followed by `open` is two syscalls on a mutable
namespace; adding a third check only moves the window.

## When this matters, and when it does not

The window is only reachable by a party who can **write into the extensions
root** between the scan and the load. That makes it a threat-model question
rather than a discovery-semantics one, and the answer differs sharply by
deployment:

| Deployment | Reachable? |
|---|---|
| Extensions root is part of the deployed artifact, read-only at runtime | **No.** Nothing can replace the link. |
| Root is operator-managed, same trust level as the host process | **No** in any useful sense — a party who can write there can replace the module file itself, and the symlink adds nothing. |
| Root is writable by a less-trusted party (a shared volume, an upload directory, a multi-tenant plugin drop) | **Yes**, and confinement alone does not hold. |

Only the third row is a real exposure, and in that row the symlink window is one
of several problems: the same writer can also drop a module that passes every
check and does whatever it likes inside `execute()`. **Confinement was never the
control protecting that deployment**, which is the main reason this is filed
rather than fixed.

## Options, with their costs

1. **Do nothing; document the boundary.** State that a writable-by-untrusted
   extensions root is outside the model, as this note does. Zero cost, zero
   coverage.
2. **Open-then-verify.** Open the file, then `fstat` the descriptor and verify
   the opened inode is the one the scan recorded, loading from the descriptor
   rather than re-opening by path. Closes the file half. Requires the loader to
   accept a descriptor, which apcore-python and apcore-typescript's import
   mechanisms do not naturally do — both import by path.
3. **Directory-descriptor traversal** (`openat` / `O_NOFOLLOW` relative walks).
   Closes the directory half too. Portable only with per-platform work, and
   Node's API surface for it is thin.
4. **Snapshot the root.** Copy or hardlink the tree to a private location, scan
   and load from there. Sidesteps the window entirely and costs a copy per
   discovery; also breaks hot reload, which watches the original tree.

## Recommendation

**Option 1 until a deployment in the third row is actually supported.** Options 2
and 3 add per-platform mechanism to close a window that only opens under a
deployment the project does not currently claim to support, and option 4
contradicts hot reload.

What should not happen is a partial version of option 2 — re-checking the path
before the load. That adds a syscall, moves the window, and leaves a comment
saying the case is handled, which is worse than the boundary being documented.

## What would change this

Any of: a supported deployment where the extensions root is writable by a party
less trusted than the host process; a hosted or multi-tenant plugin story; or a
loader that already takes a descriptor, which would make option 2 nearly free for
the file half.
