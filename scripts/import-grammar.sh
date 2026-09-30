#!/usr/bin/env bash
set -euo pipefail

if [[ ${1:-} == --help ]]; then
    cat <<'HELP'
Usage: scripts/import-grammar.sh [gd fr it ga cy es]

Import unfinished portions of eligible PDFs in the selected languages on calypso.
With no arguments, process gd fr it ga cy es. SOURCES.md controls documents
and levels. There is no total rule limit; existing rules are preserved and
new proposals are checked for duplicates. PostgreSQL records progress by
file contents and level, so reruns skip completed work and resume failures.
Source extraction and rule comparisons are cached in PostgreSQL. Syllabus growth
reuses extracted points and checks only unseen rule pairs; retries reuse results
saved before a later failure. Previously uncached work still calls the model.
The first run also checks old books without progress records for missed rules.
This calls the configured model and writes grammar rules. Full output goes to logs/.
Requires a deployed wisecrow binary supporting import-pdf --incremental.
HELP
    exit 0
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
log="logs/grammar-import-$(date -u +%Y%m%dT%H%M%SZ)-$$.log"
printf 'Full import log: %s/%s\n' "$PWD" "$log"
trap 'printf "Import stopped. See %s; rerun this script to resume.\n" "$log" >&2' ERR
compose=(docker compose -f docker-compose.deploy.yml)

# Validate the whole selection before spending model calls on any language.
"${compose[@]}" exec -T wisecrow-web test -r /app/grammar/SOURCES.md
for language in "${languages[@]}"; do
    "${compose[@]}" exec -T wisecrow-web test -d "/app/grammar/$language"
done

for language in "${languages[@]}"; do
    printf '\nImporting unfinished %s documents at their recorded levels\n' "$language" | tee -a "$log"
    # Omitting --level lets the importer keep levels ordered and extract each
    # document once; an explicit six-level loop would also admit unlevelled books.
    "${compose[@]}" exec -T wisecrow-web wisecrow import-pdf --incremental \
        --file "/app/grammar/$language" --lang "$language" 2>&1 \
        | tee -a "$log" \
        | awk '/ INFO| WARN| ERROR|^[Ee]rror/ { print; fflush() }'
done

printf '\nCoverage after import (zero rows included; no rule ceiling):\n' | tee -a "$log"
"${compose[@]}" exec -T postgres sh -c \
    "exec psql -X -v ON_ERROR_STOP=1 -P pager=off -U \"\$POSTGRES_USER\" -d \"\$POSTGRES_DB\"" \
    <<'SQL' 2>&1 | tee -a "$log"
BEGIN READ ONLY;
WITH requested(code, position) AS (
    VALUES ('gd', 1), ('fr', 2), ('it', 3), ('ga', 4), ('cy', 5), ('es', 6)
)
SELECT requested.code AS language, cl.code AS level,
       COUNT(gr.id) FILTER (WHERE gr.source = 'pdf') AS document_rules,
       COUNT(gr.id) AS total_rules
FROM requested
CROSS JOIN cefr_levels cl
LEFT JOIN languages l ON l.code = requested.code
LEFT JOIN grammar_rules gr ON gr.language_id = l.id AND gr.cefr_level_id = cl.id
WHERE cl.code IN ('A1', 'A2', 'B1', 'B2', 'C1', 'C2')
GROUP BY requested.code, requested.position, cl.code, cl.sort_order
ORDER BY requested.position, cl.sort_order;
COMMIT;
SQL
printf '\nImport commands finished. Review duplicate, Skipped and Refused lines in %s.\n' "$log"
