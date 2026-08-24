# AGENTS.md

## Architecture

* Keep the codebase modular. Avoid monolithic files, components, or modules.
* Follow Rust best practices and prefer simple, readable, maintainable code.
* Keep source files under 300 lines where practical.
* A small exception up to roughly 350 lines is acceptable when splitting the file would make the code less clear.
* If existing code becomes too large or mixes unrelated responsibilities, refactor it into smaller focused modules.
* Prefer clear separation between UI, recording, input tracking, media, timeline, and project logic.
* Avoid unnecessary abstractions. Introduce them only when they simplify the codebase or reduce duplication.

## Code Quality

* Keep functions focused on a single responsibility.
* Prefer descriptive names over excessive comments.
* Add documentation where behavior, architecture, or non-obvious decisions need explanation.
* Do not add comments that simply restate the code.
* Remove dead code, unused imports, temporary debugging code, and unnecessary dependencies.
* Reuse existing utilities and patterns before introducing new ones.

## Changes

* Keep changes scoped to the task being implemented.
* Do not perform unrelated rewrites unless required to keep the architecture clean.
* When touching problematic or oversized code, improve its structure where reasonable without expanding the task unnecessarily.
* Preserve existing behavior unless the task explicitly requires changing it.

## Validation

Before considering a change complete:

* Run `cargo fmt`.
* Run `cargo check`.
* Run relevant tests.
* Run Clippy when practical and resolve warnings introduced by the change.
