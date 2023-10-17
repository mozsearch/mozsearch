#!/usr/bin/env bash

SCRIPT_DIR=$(dirname $0)

PARSE_EXPR='parse "* + /home/ubuntu/mozsearch/scripts/* *" '
PARSE_EXPR+=' as time, script, args'
PARSE_EXPR+=' | parseDate(time) as time'
PARSE_EXPR+=' | split(args) on " "'
PARSE_EXPR+=' | if(script == "find-repo-files.py" or script == "build.sh" or script=="output.sh", args[2], "") as tree'
PARSE_EXPR+=' | if(script == "js-analyze.sh" or script == "java-analyze.sh" or script == "idl-analyze.sh" or script == "ipdl-analyze.sh" or script == "crossref.sh" or script == "build-codesearch.py" or script == "check-index.sh" or script == "compress-outputs.sh" or script == "check-index.sh", args[1], tree) as tree'

# - Grep the log in Perl mode looking for the pattern where a "date" invocation
#   is followed by a script invocation from mozsearch/scripts.
#   - We use `-P` to get fancy Perl mode
#   - We use `-a` to force ASCII mode so it doesn't decide it's a binary file.
#   - We use `-z` so that grep sees a single giant line, which combined with
#     `-o` only outputs what matched.  We use a look-behind assertion so that
#     we can match on the `+ date` line but not include it in the output.
#   - We use `-h` to suppress the filename
# - We use `paste` to join these consecutive lines.
# - We use `tr -d '\0'` to eat a leading nul that ends up in there at the start
#   of the lines.
#
# The net output looks like:
# Sat Oct  2 04:41:33 UTC 2021 + /home/ubuntu/mozsearch/scripts/find-repo-files.py /home/ubuntu/config /mnt/index-scratch/config.json nss
# Sat Oct  2 04:41:35 UTC 2021 + /home/ubuntu/mozsearch/scripts/build.sh /home/ubuntu/config /mnt/index-scratch/config.json nss
# Sat Oct  2 04:41:35 UTC 2021 + /home/ubuntu/mozsearch/scripts/indexer-setup.py
grep -Pazoh "(?<=\n\+ date\n)[^\n]+\n\+ /home/ubuntu/mozsearch/scripts/[^\n]+\n" index-* \
  | paste -d" " - - \
  | tr -d '\0' \
  | agrind --output json "* | ${PARSE_EXPR}" \
  | ${SCRIPT_DIR}/indexer-logs-print.py
