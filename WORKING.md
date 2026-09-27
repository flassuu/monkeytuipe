# How to work on this, fast

Written after a round of work where a handful of small changes took hours. The
measurements are at the bottom; the rules come from them.

## The diagnosis, in one line

The toolchain is not slow. A full green gate on cached artifacts is **16
seconds**. Roughly 90% of the time went into self-inflicted compile-fix cycles,
things I built twice, tests coupled to the wrong thing, and questions I did not
ask.

## The rules

### 0. Check the premise before building (≤5 min)

If a request rests on a fact about the upstream site or its API, **verify it
first**, then report, then build.

The expensive miss: "add an interface language" was asked as a port from
monkeytype. The site has no i18n at all — no translation files, no module, no
dependency, and a maintainer saying so in public. That took ten minutes to
establish and I built 525 lines plus eight touched files anyway. Five minutes of
checking would have turned "there is nothing to port, here is what I propose" into
a question instead of an assumption.

### 1. Two readings → one question

If two readings of a requirement produce different code, ask. List them. Do not
pick silently, and do not pick after building.

Three times this round there was a genuine fork and three times I chose alone:

- the top bar's active style (I had argued for a fill the round before)
- the interface language, given no upstream to copy
- whether the settings screen should be restyled or merely restyled *differently*

A wrong guess on a fork is not a bug, it is a wasted feature.

### 2. Map the blast radius before the first edit

`rg` the symbol. Count the call sites. **More than eight is a design change, not
an edit.** Thirty seconds, and it is the difference between one edit and twenty.

### 3. Write the shape down first

The structs, the invariants, the layout rule. Ten lines, in the commit message or
a comment. Then implement once.

This is the rule that kills rewrites. The top bar is now 1373 lines with `Card`,
`Metrics`, `Placement`, `Attempt` and `Block`; `Button.label` went from
`&'static str` (with a `Box::leak` helper to make that work) to `String`; and the
width question grew four separate functions — `total_width`, `narrowest`,
`packed_width`, `two_row_minimum` — one per new question, each with its own tests.
Starting with `String` labels, three cards, and one width function would have made
two of those four rewrites disappear.

### 4. No blind edits

Every scripted `replace` must **assert its anchor** and print what it changed. A
replacement that reports *0 sites* is a failure, not a no-op — one of mine did,
and I moved on.

Prefer the edit tool with real surrounding context. It fails loudly at the right
place. A scripted `replace` fails silently, or matches the doc comment instead of
the code, or applies twice.

Damage from this in one round: a function inserted twice and the file mangled; a
doc-comment anchor matched instead of the code; an `impl` block that landed after
the test module; a method that landed in the wrong `impl`; a `Key::Words` that did
not exist. Each cost a confusing error, a read, and a fix.

### 5. See it, then assert it

A throwaway `TestBackend` print takes twenty seconds and shows the truth. Write
assertions **after** seeing it.

The word-colour bug is the example that should be the default: I had a theory
(something in the renderer marks correct words red), printed the buffer with its
cell colours, and disproved my own theory in one run. Twenty seconds. The
alternative — reasoning about it — is what produced a confident report that the
behaviour was fine when I had not looked.

### 6. Narrow tests, one gate

| command | cached | cold |
|---|---|---|
| `cargo check -q` | 0.9s | 60s |
| `cargo test -q --lib <module>` | 2.3s | 83s |
| `cargo test -q` (all 611) | 8.5s | 22s |
| `cargo clippy -q --all-targets --all-features` | 8.0s | 8.2s |

- `cargo check` after every edit. Not `cargo build` — no codegen needed.
- `cargo test --lib <module>` per module you touched.
- **One** full `fmt + clippy + test` per commit.
- **Never `cargo clean` mid-work.** That is 2.4 minutes and it makes the next
  three commands slow too.

Running all 611 after every small edit was pure waste: about forty full runs,
eleven minutes, to learn things a two-second targeted run would have said.

### 7. Tests describe behaviour, not structure

Every test that broke for the wrong reason was coupled to the implementation.

- `a_bar_too_narrow_is_not_drawn_at_all` asserted widths `0..60` — a constant
  that was in the *code*, not in the requirement. It broke when the two-row form
  landed, and the fix was a new test rather than a new understanding.
- `the_modes_do_not_move_between_modes` asserted the wrong quantity and only then
  revealed that centring the whole cluster — not the test — was the bug.

Ask of every assertion: *would this still be true if the code were written
differently and correctly?* If not, it is a change detector, and it will fire on
the next good refactor.

### 8. The commit message describes the diff

Write it **last**, from `git diff --stat`.

One message this round described a settings restyle that the diff did not contain,
and the next commit had to do the work. A commit message written from intent
instead of from the diff is a claim about code that does not exist, which is the
worst kind of documentation.

### 9. Parallelise the read-only work

Subagents for: checking a premise, mapping call sites, drafting a test list from a
spec. All read-only, all independent of what I am editing. One ran here and found
the i18n answer in ten minutes; two more would have covered the call-site mapping
and the render pipeline at the same time.

## The budget

A change confined to one module should go from *reading* to *green with one full
gate* in **under five minutes** of wall clock.

Slower than that means a rule above was broken. Usually it is **3** (built it
twice) or **4** (a blind edit cost a cycle).

## What to do when it goes wrong anyway

Say so in the commit message, with the measurement. The commit that added the
two-row bar fallback says the Russian quote bar needed 90 columns where English
needs 77, and that is a fact a reader can check — not "improved the layout".

## The measurements

Linux, warm filesystem, `target/` populated.

```
$ touch src/i18n.rs && cargo check -q          936ms
$ touch src/i18n.rs && cargo test -q --lib i18n  2335ms
$ touch src/app.rs  && cargo test -q --lib        9076ms
$ cargo check -q                        (cached)  253ms
$ cargo test -q                         (cached) 7705ms
$ cargo clippy -q --all-targets         (cached) 7985ms
$ cargo clean && cargo check -q                60578ms
$ cargo clean && cargo test -q --lib          83213ms
```

The conclusion is not "the machine is slow". It is that a 16-second gate was being
used to check a 20-second question, and the real time was going into getting the
edit right the first time.
