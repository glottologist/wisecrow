#!/usr/bin/env bash
# One corpus step inside wisecrow-web. Sleeps only when the step asked to wait,
# so a finished archive is followed at once and a quiet pair does not spin.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
compose=(docker compose -f docker-compose.deploy.yml)

while true; do
    output=$("${compose[@]}" exec -T wisecrow-web \
        wisecrow prepare-corpus --config /app/language-pairs.toml)
    printf '%s\n' "$output"
    # The binary also logs to stdout. The step word is the last line that is
    # exactly one of the three results.
    step=$(printf '%s\n' "$output" | awk '/^(worked|idle|retry)$/ { found=$0 } END { print found }')
    # retry is the same kind of wait as idle: the failure was not recorded, and
    # repeating it with no pause would hammer the host that just failed.
    case "$step" in
        worked) ;;
        idle|retry) sleep 60 ;;
        *) printf 'prepare-corpus returned no step line\n' >&2; exit 1 ;;
    esac
done
