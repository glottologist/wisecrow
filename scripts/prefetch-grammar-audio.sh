#!/usr/bin/env bash
set -euo pipefail

if [[ ${1:-} == --help ]]; then
    cat <<'HELP'
Usage: scripts/prefetch-grammar-audio.sh [--preview] [gd fr it ga cy es]

Generate missing example clips, then verify each language has no missing,
failed or unsupported clips. With no languages, process gd fr it ga cy es.
--preview only reports current coverage, without generating any audio.
Successful clips are reused on a rerun. Full output goes to logs/.
HELP
    exit 0
fi

preview_only=false
if [[ ${1:-} == --preview ]]; then
    preview_only=true
    shift
fi
languages=("$@")
if (( ${#languages[@]} == 0 )); then
    languages=(gd fr it ga cy es)
fi
for language in "${languages[@]}"; do
    case "$language" in
        gd|fr|it|ga|cy|es) ;;
        *) printf 'Unsupported language: %s\n' "$language" >&2; exit 2 ;;
    esac
done

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
mkdir -p logs
exec 9>logs/grammar.lock
if ! flock -n 9; then
    printf 'Another grammar import or audio script is running.\n' >&2
    exit 1
fi
log="logs/grammar-audio-$(date -u +%Y%m%dT%H%M%SZ)-$$.log"
printf 'Audio log: %s/%s\n' "$PWD" "$log"
trap 'printf "Audio run stopped. See %s; rerun this script to resume.\n" "$log" >&2' ERR
compose=(docker compose -f docker-compose.deploy.yml)

for language in "${languages[@]}"; do
    if [[ $preview_only == false ]]; then
        printf '\nGenerating %s grammar audio\n' "$language" | tee -a "$log"
        "${compose[@]}" exec -T wisecrow-web wisecrow prefetch-grammar-audio \
            --lang "$language" 2>&1 | tee -a "$log"
    fi

    if preview=$("${compose[@]}" exec -T wisecrow-web wisecrow prefetch-grammar-audio \
        --lang "$language" --dry-run 2>&1); then
        printf '%s\n' "$preview" | tee -a "$log"
    else
        printf '%s\nPreview failed for %s.\n' "$preview" "$language" | tee -a "$log" >&2
        exit 1
    fi

    if [[ $preview_only == false ]] && ! grep -Eq \
        "Previewed [0-9]+ $language example sentences \([0-9]+ distinct\): cached [0-9]+, missing 0, generated 0, failed 0, unsupported 0;" \
        <<<"$preview"; then
        printf 'Audio coverage is incomplete or its summary is absent for %s. See %s.\n' \
            "$language" "$log" >&2
        exit 1
    fi
done

if [[ $preview_only == true ]]; then
    printf '\nPreview finished. See %s for coverage.\n' "$log"
else
    printf '\nAudio verified for every selected language. Log: %s\n' "$log"
fi
