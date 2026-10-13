#!/usr/bin/env bash
# Asks grammar-status before any model or speech call. Exit 0 is idle.
# Exit 10 names the configured languages that still have schedule or clip work.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
compose=(docker compose -f docker-compose.deploy.yml)

set +e
output=$("${compose[@]}" exec -T wisecrow-web \
    wisecrow grammar-status --config /app/language-pairs.toml --shelf /app/grammar)
code=$?
set -e
printf '%s\n' "$output"
if [[ $code -eq 0 ]]; then
    sleep 60
    exit 0
fi
if [[ $code -ne 10 ]]; then
    printf 'grammar-status exited %s\n' "$code" >&2
    exit "$code"
fi

read -r -a languages <<<"${output#work }"
scripts/import-grammar.sh "${languages[@]}"
scripts/prefetch-grammar-audio.sh "${languages[@]}"
