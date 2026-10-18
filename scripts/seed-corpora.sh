#!/usr/bin/env bash
# Seed the corpora of the Bolero fuzz targets that take raw JSON bytes with
# the workspace's JSON fixtures and JSONTestSuite (Plan 4, K6). Run from the
# workspace root before `cargo bolero test`; the nightly fuzz job does.
#
# Bolero keeps a target's corpus in `__fuzz__/<module path>/corpus/` next to
# the file that defines it, with `::` as `__` and any `_fuzz` suffix
# dropped. `cargo test` replays every corpus file too, so a seeded checkout
# runs these inputs as ordinary tests. The seeds are copies, so they are
# not committed (see .gitignore).
set -euo pipefail

cd "$(dirname "$0")/.."

json_sources=(tests/valid_json_fixtures tests/json_test_suite/test_parsing)

seed() {
    local dir="$1/corpus"
    shift
    mkdir -p "$dir"
    local src file
    for src in "$@"; do
        for file in "$src"/*.json; do
            # Prefix with the source directory, so names cannot collide.
            cp "$file" "$dir/$(basename "$src")__$(basename "$file")"
        done
    done
    echo "$dir: $(ls "$dir" | wc -l | tr -d ' ') files"
}

core=src/verify/__fuzz__
for target in parser_never_panics context_invariants_hold accepts_only_valid_json \
    accepts_all_valid_json events_match_reference; do
    seed "$core/verify__state_machine__$target" "${json_sources[@]}"
done
seed "$core/verify__ref_lexer__ref_lexer_matches_logos" "${json_sources[@]}"
seed "$core/verify__tests__utf8_check_matches_std" tests/json_test_suite/test_parsing

seed yajlish-ndjson/src/__fuzz__/verify__ndjson_never_panics \
    yajlish-ndjson/tests/ndjson_fixtures "${json_sources[@]}"
