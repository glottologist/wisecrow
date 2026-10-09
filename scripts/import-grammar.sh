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
A language that fails is logged and the rest still run; the script then exits 1.
A usage limit pauses until the reset time named in the provider message, or
for two hours when that message names none, then the same language resumes.
A lapsed login or an outage still stops the script at once with exit status 75.
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

# The importer exits with this status when the model provider cannot answer.
# A usage limit names when it lifts; a lapsed login or an outage does not, and
# every later call would fail the same way, so only those stop the loop.
llm_unavailable=75
# Past the stated reset, so a retry does not land on the second the window opens.
limit_reset_buffer=120
limit_fallback_seconds=$((2 * 60 * 60))
limit_max_seconds=$((8 * 24 * 60 * 60))
failed=()

# Print how long a pause is, as "2h 5m" or "2d 3h".
format_pause() {
    local seconds=$1
    local hours=$((seconds / 3600))
    local minutes=$(((seconds % 3600) / 60))
    if (( hours >= 48 )); then
        printf '%dd %dh' $((hours / 24)) $((hours % 24))
    elif (( hours > 0 && minutes > 0 )); then
        printf '%dh %dm' "$hours" "$minutes"
    elif (( hours > 0 )); then
        printf '%dh' "$hours"
    else
        printf '%dm' "$minutes"
    fi
}

# Unix time of the reset named in one provider error, plus the buffer.
# Returns 1 when the line names no usable time.
seconds_until_reset() {
    local line=$1
    local resets_re='resets[[:space:]]+(.*)$'
    if [[ ! $line =~ $resets_re ]]; then
        return 1
    fi
    local spec=${BASH_REMATCH[1]}
    spec=${spec%$'\r'}
    spec=${spec%"${spec##*[![:space:]]}"}
    spec=${spec%.}
    local tz=UTC
    local clock=$spec
    local tz_re='^(.*)[[:space:]]+\(([^)]+)\)$'
    if [[ $spec =~ $tz_re ]]; then
        clock=${BASH_REMATCH[1]}
        tz=${BASH_REMATCH[2]}
        clock=${clock%"${clock##*[![:space:]]}"}
    fi
    local epoch
    if ! epoch=$(TZ="$tz" date -d "$clock" +%s 2>/dev/null); then
        return 1
    fi
    local now
    now=$(date -u +%s)
    if (( epoch <= now )); then
        local rolled
        if [[ $clock =~ ^(Mon|Tue|Wed|Thu|Fri|Sat|Sun) ]]; then
            if ! rolled=$(TZ="$tz" date -d "$clock + 7 days" +%s 2>/dev/null); then
                return 1
            fi
        elif ! rolled=$(TZ="$tz" date -d "$clock + 1 day" +%s 2>/dev/null); then
            return 1
        fi
        epoch=$rolled
    fi
    local wait=$((epoch + limit_reset_buffer - now))
    if (( wait < 30 || wait > limit_max_seconds )); then
        return 1
    fi
    printf '%s\n' "$wait"
}

# Seconds to pause when the new log bytes name a usage limit. Returns 1 when
# they do not, which is a lapsed login or an outage.
usage_limit_pause_seconds() {
    local log=$1
    local offset=$2
    local line
    line=$(tail -c +"$((offset + 1))" "$log" \
        | grep -E -i 'session limit|weekly limit|usage limit|5-hour limit|hit your .+ limit' \
        | tail -n 1 || true)
    if [[ -z $line ]]; then
        return 1
    fi
    local seconds
    if seconds=$(seconds_until_reset "$line"); then
        printf '%s\n' "$seconds"
    else
        printf '%s\n' "$limit_fallback_seconds"
    fi
}

for language in "${languages[@]}"; do
    while true; do
        printf '\nImporting unfinished %s documents at their recorded levels\n' "$language" | tee -a "$log"
        # Omitting --level lets the importer keep levels ordered and extract each
        # document once; an explicit six-level loop would also admit unlevelled books.
        log_offset=$(stat -c %s "$log")
        status=0
        "${compose[@]}" exec -T wisecrow-web wisecrow import-pdf --incremental \
            --file "/app/grammar/$language" --lang "$language" 2>&1 \
            | tee -a "$log" \
            | awk '/ INFO| WARN| ERROR|^[Ee]rror/ { print; fflush() }' \
            || status=$?
        if (( status == llm_unavailable )); then
            if pause_seconds=$(usage_limit_pause_seconds "$log" "$log_offset"); then
                resume=$(date -u -d "now + ${pause_seconds} seconds" '+%Y-%m-%d %H:%M UTC')
                printf '\nUsage limit while importing %s. Pausing until %s (%s), then resuming.\n' \
                    "$language" "$resume" "$(format_pause "$pause_seconds")" \
                    | tee -a "$log" >&2
                sleep "$pause_seconds"
                continue
            fi
            printf '\nThe model provider cannot answer (login or outage); stopped at %s.\n' \
                "$language" | tee -a "$log" >&2
            printf 'Import stopped. See %s; rerun this script to resume.\n' "$log" >&2
            exit "$llm_unavailable"
        elif (( status != 0 )); then
            printf 'Import of %s failed with exit status %d; continuing with the next language.\n' \
                "$language" "$status" | tee -a "$log" >&2
            failed+=("$language")
        fi
        break
    done
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
if (( ${#failed[@]} > 0 )); then
    printf 'Unfinished languages: %s. Rerun this script to resume them.\n' "${failed[*]}" | tee -a "$log" >&2
    exit 1
fi
