# tesla-superchargers verification map

This directory is the maintained source for verifying operator-facing behavior of tesla-superchargers. Read this index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Launch with `.cursor/skills/verify-tesla-superchargers/scripts/launch.sh <run_id>` unless a recipe asks for `--empty`.
- Run `scripts/doctor.sh <run_id>` and require a healthy `host` that this run started.
- Put proof under `.cursor/skills/verify-tesla-superchargers/evidence/<run_id>/`.
- Never drive a `host` or database that this run did not create.
- Do not run `scrape` or `retry-failed`. Those commands open Chrome and call Tesla.

## Driving conventions

- Start every recipe from the baseline state unless its preconditions say otherwise.
- Drive HTTP through `scripts/http.sh`. Drive CLI through `scripts/cli.sh`.
- Treat every command as literal. Keep ids, routes, and flags unchanged.
- Restore fixture data after a mutation that is not the point of the recipe. Do not remove proof artifacts during cleanup.

## Proof and skip reporting

- Capture the user action and the resulting state, not only the last response.
- HTTP proof includes status code and JSON body.
- CLI proof includes the command, stdout, stderr, and exit code.
- Mutation proof includes a read-only second view of the stored value.
- Record the feature id and entry point used with every artifact.
- Report an unreachable path with the attempted command and the unmet precondition.
- Do not report a skipped entry point as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with verify-tesla-superchargers` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

Keep implementation details out of the map. Name only user paths, stable handles, required state, commands, and observable proof.

## Features

- [CLI status](./status.md) covers empty and seeded `status` output.
- [Read API](./read-api.md) covers health, list, filters, stats, map, detail, and recent feeds.
- [Import scrapes](./import.md) covers snapshot and diff import plus admin auth.
- [Export](./export.md) covers `export-snapshot` and `export-diff` files.
- [Live scrape](./scrape.md) is unreachable here. It needs Chrome and Tesla's Find Us API.
