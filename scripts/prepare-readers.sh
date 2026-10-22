#!/usr/bin/env bash
# Asks reader-status before any model or speech call. Exit 0 is idle.
# Exit 10 means one prepare-readers step is due. Exit 75 is a model outage.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$script_dir/.."
compose=(docker compose -f docker-compose.deploy.yml)

llm_unavailable=75
limit_reset_buffer=120
limit_fallback_seconds=$((2 * 60 * 60))
limit_max_seconds=$((8 * 24 * 60 * 60))
mkdir -p logs
log="logs/prepare-readers.log"
touch "$log"

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

while true; do
    set +e
    output=$("${compose[@]}" exec -T wisecrow-web \
        wisecrow reader-status --config /app/language-pairs.toml --shelf /app/readers)
    code=$?
    set -e
    printf '%s\n' "$output"
    line=$(printf '%s\n' "$output" | awk '/^(idle|work)$/ { found=$0 } END { print found }')
    if [[ $code -eq 0 && $line == idle ]]; then
        sleep 60
        continue
    fi
    if [[ $code -ne 10 || $line != work ]]; then
        printf 'reader-status exited %s\n' "$code" >&2
        if [[ $code -eq 0 ]]; then
            exit 1
        fi
        exit "$code"
    fi

    log_offset=$(stat -c %s "$log")
    status=0
    "${compose[@]}" exec -T wisecrow-web \
        wisecrow prepare-readers --config /app/language-pairs.toml --shelf /app/readers \
        2>&1 | tee -a "$log" || status=$?
    if (( status == llm_unavailable )); then
        if pause_seconds=$(usage_limit_pause_seconds "$log" "$log_offset"); then
            resume=$(date -u -d "now + ${pause_seconds} seconds" '+%Y-%m-%d %H:%M UTC')
            printf '\nUsage limit. Pausing until %s (%s), then resuming.\n' \
                "$resume" "$(format_pause "$pause_seconds")" | tee -a "$log" >&2
            sleep "$pause_seconds"
            continue
        fi
        exit 75
    elif (( status != 0 )); then
        exit "$status"
    fi
done
