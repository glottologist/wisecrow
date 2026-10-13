#!/usr/bin/env bash
# One corpus step inside wisecrow-web. Sleeps only when the step asked to wait,
# so a finished archive is followed at once and a quiet pair does not spin.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
compose=(docker compose -f docker-compose.deploy.yml)

output=$("${compose[@]}" exec -T wisecrow-web \
    wisecrow prepare-corpus --config /app/language-pairs.toml)
printf '%s\n' "$output"
# retry is the same kind of wait as idle: the failure was not recorded, and
# repeating it with no pause would hammer the host that just failed.
case "$output" in
    idle|retry) sleep 60 ;;
esac
