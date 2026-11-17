# Working on Corroborate

Rules for anyone changing this repository, people or AI agents. What the tool is for: the
[main README](README.md). Where code lives: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Project

- Dev stage: no backwards compatibility. Change a data model, port, JSON form or project format
  outright; no migrations, aliases or "old format" readers. Old compatibility code may be removed
  when you touch it.
- Leave `(leave this alone mr claude code).txt` in the root alone.
- Commit straight to `main`, no branches or pull requests. Push only when asked. Git has no global
  identity on this PC: use the author from `git log` (`git -c user.name=… -c user.email=…`).
- Another session may share the checkout: stage files by path, never `git add -A`.

## Design

- Times and locations travel only on wires: no project-wide pool, no global solve.
- Every value carries its error (1 sigma, combined as root-sum-square) and its source.
- A module is a folder (`src/modules/<Label>/`: `module.toml`, README, code). A type is
  `src/types/<id>/type.toml`; its JSON form is in `src/data.rs`.
- Nodes have no windows: each draws itself on the canvas. Settings are the only way the canvas
  changes processing.
- Tools show evidence and its uncertainty; people decide. Never hide a contradiction.
- egui/eframe 0.36.

## Docs

- One home per fact: the table in [docs/module-docs/plan.md](docs/module-docs/plan.md#one-home-per-fact)
  says which file owns what. Link to it from elsewhere; don't copy it.
- Update the docs in the same commit as the code they describe: ports, types, JSON, settings,
  behaviour. Check the relative links in every .md you touch, and that a new module is in the
  main README's module table.
- Ask before creating a new .md file. Add to the file that owns the fact instead.
- A module README follows [the Template](src/modules/Template/README.md): Inputs, Steps,
  Outputs, Limits, Files, Tests. Every Tested cell says what it was tested on, or "not yet".
- Plans and their dated decisions go in `docs/<feature>/plan.md`; READMEs link to them.
- Write plainly, in an investigator's words, in short sentences. Say what is not built yet.
- Each source file starts with a comment saying what it holds.
- Mention doc problems you notice in passing, even if you don't fix them.

## Building and testing

- `run.cmd` builds and starts the workbench from `app\`. Needs Rust, ffmpeg and yt-dlp on `PATH`
  (README, Build and run).
- Run only the tests near your change, by name:
  `cargo test --profile fast --all-features --bin corroborate -- <name>` (or `--lib <module>`).
  Build once first. The full suite takes many minutes: run it only when asked.
- Don't rewrite files in bulk (`sed -i` over a folder): new timestamps and line endings force a
  full rebuild. Edit only the files you mean to change.
- Five workbench tests need `cargo build --profile fast --example bus_echo` first.
- Say which tests you ran, and what you did not check.
