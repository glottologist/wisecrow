#!/usr/bin/env bash
# Asks grammar-status before any model or speech call. Exit 0 is idle.
# Exit 10 names the configured languages that still have schedule or clip work.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
compose=(docker compose -f docker-compose.deploy.yml)

while true; do
    set +e
    output=$("${compose[@]}" exec -T wisecrow-web \
        wisecrow grammar-status --config /app/language-pairs.toml --shelf /app/grammar)
    code=$?
    set -e
    printf '%s\n' "$output"
    # Startup logs share stdout with the status line. Keep the last idle/work line.
    line=$(printf '%s\n' "$output" | awk '/^(idle|work([[:space:]].*)?)$/ { found=$0 } END { print found }')
    if [[ $code -eq 0 && $line == idle ]]; then
        sleep 60
        continue
    fi
    if [[ $code -ne 10 || $line != work* ]]; then
        printf 'grammar-status exited %s\n' "$code" >&2
        if [[ $code -eq 0 ]]; then
            exit 1
        fi
        exit "$code"
    fi

    read -r -a languages <<<"${line#work }"
    if (( ${#languages[@]} == 0 )); then
        printf 'grammar-status named no language\n' >&2
        exit 1
    fi
    scripts/import-grammar.sh "${languages[@]}"
    scripts/prefetch-grammar-audio.sh "${languages[@]}"
done
