#!/usr/bin/env bash

# Arguments: <log path>

# This script uses https://github.com/rcoh/angle-grinder in order to process our
# searchfox logs from /index/logs/searchfox.log
#
# Install/update angle-grinder via `cargo install ag`

# We don't want to show commands, so no: set -x
set -eu # Errors/undefined vars are fatal
# We intentionally induce a pipe error in our use of head, so no: set -o pipefail

# Core parsing
PARSE_EXPR='parse "[*] [Cache:*] [*] [*] [Remote_Addr: *]'
PARSE_EXPR+=' - * - * to: *: \"GET /*/* HTTP/1.1\" * * \"*\" \"*\"" '
PARSE_EXPR+='as time_local, cache_status, request_time, host, remote_addr,'
PARSE_EXPR+=' remote_user, server_name, upstream_addr, repo, repo_path, status,'
PARSE_EXPR+=' body_bytes_sent, referrer, user_agent'
# But we would like to be able to extract the action from the repo_path which
# could look like "rev/MOREPATH" or "search?query".
PARSE_EXPR+=' | substring(repo_path, 0, 6) as maybe_search'
PARSE_EXPR+=' | maybe_search == "search" as is_search'

if [[ ${2:-} ]]; then
  MAYBE_REPO_FILTER="where repo == \"$2\" | "
  MAYBE_REPO_LABEL=" for $2"
else
  MAYBE_REPO_FILTER=""
  MAYBE_REPO_LABEL=""
fi

CACHE_CHECK='where cache_status != "-"'
MISS_CHECK='where cache_status == "MISS"'

GET_ACTION='where !is_search | parse "*/*" from repo_path as action, path'
ONLY_SEARCH='where is_search'

STATS='count, p50(request_time), p66(request_time), p75(request_time), p90(request_time), p95(request_time), p99(request_time)'

# This includes 2 lines of headers.
SLOW_COUNT=12

## Output dynamic request latencies

echo "### Dynamic Non-Search Request Latencies${MAYBE_REPO_LABEL}"
echo ''
echo '```'
cat $1 | agrind "* | ${PARSE_EXPR} |${MAYBE_REPO_FILTER} ${CACHE_CHECK} | ${GET_ACTION} | ${STATS} by action, cache_status"
echo '```'

echo ''
echo "### Dynamic Search Request Latencies${MAYBE_REPO_LABEL}"
echo ''
echo '```'
cat $1 | agrind "* | ${PARSE_EXPR} |${MAYBE_REPO_FILTER} ${CACHE_CHECK} | ${ONLY_SEARCH} | ${STATS} by cache_status"
echo '```'


echo ''
echo "### Slowest Searches${MAYBE_REPO_LABEL}"
echo ''
echo '```'
# agrind supports a limit operator but it appears there's a buggy optimization
# which ends up performing the limit prior to the sort happening.  This doesn't
# appear to be a string/numeric mismatch as things still happen if I try and
# force a coercion.
#
# So we just pipe the output through head.  Actually, we pipe it through tac
# first so that agrind doesn't emit an error about the closed pipe.
cat $1 | agrind "* | ${PARSE_EXPR} |${MAYBE_REPO_FILTER} ${MISS_CHECK} | ${ONLY_SEARCH} | sort by request_time desc | fields + request_time, repo_path" | tac | tail -n${SLOW_COUNT} | tac
echo '```'


echo ''
echo "### Slowest Rev Requests${MAYBE_REPO_LABEL}"
echo ''
echo '```'
# agrind supports a limit operator but it appears there's a buggy optimization
# which ends up performing the limit prior to the sort happening.  This doesn't
# appear to be a string/numeric mismatch as things still happen if I try and
# force a coercion.
#
# So we just pipe the output through head.  Actually, we pipe it through tac
# first so that agrind doesn't emit an error about the closed pipe.
cat $1 | agrind "* | ${PARSE_EXPR} |${MAYBE_REPO_FILTER} ${MISS_CHECK} | ${GET_ACTION} | sort by request_time desc | fields + request_time, path, action" | tac | tail -n${SLOW_COUNT} | tac
echo '```'
