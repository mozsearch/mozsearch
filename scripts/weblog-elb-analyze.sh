#!/usr/bin/env bash

# Arguments: <log path>

# This script uses https://github.com/rcoh/angle-grinder in order to process our
# searchfox load-balancer logs, which can be retrieved using weblog-elb-fetch.sh.
#
# Install/update angle-grinder via `cargo install ag`

# We don't want to show commands, so no: set -x
set -eu # Errors/undefined vars are fatal
# We intentionally induce a pipe error in our use of head, so no: set -o pipefail

# Core parsing
# But we would like to be able to extract the action from the repo_path which
# could look like "rev/MOREPATH" or "search?query".
#PARSE_EXPR+=' | substring(repo_path, 0, 6) as maybe_search'
#PARSE_EXPR+=' | maybe_search == "search" as is_search'

PARSE_EXPR='parse "* * * *:* *:* * * * * * * * \"*\" \"*\" * * * \"*\" \"*\" \"*\"'
PARSE_EXPR+=' * * \"*\" \"*\" \"*\" \"*\" \"*\" \"*\" \"*\"" '
PARSE_EXPR+='as type, time, elb, client_ip, client_port, target_ip, target_port,'
PARSE_EXPR+=' request_processing_secs, target_processing_secs, response_processing_secs,'
PARSE_EXPR+=' elb_status_code, target_status_code, received_bytes, sent_bytes,'
PARSE_EXPR+=' request, user_agent,'
PARSE_EXPR+=' ssl_cipher, ssl_protocol, target_group_arn,'
PARSE_EXPR+=' trace_id, domain_name, chosen_cert_arn, matched_rule_priority,'
PARSE_EXPR+=' request_creation_time, actions_executed, redirect_url, error_reason,'
PARSE_EXPR+=' target_list, target_status_code_list, classification, classification_reason'
PARSE_EXPR+=' | parse "* *://*:*/* *" from request as req_method, req_scheme, req_host, req_port, req_path, req_protocol nodrop'

# bot-wise, ignore:
# - search engine bots that helpfully have "bot" or "Bot" in the name.
# - bots that self-identify by fetching robots.txt and I manually added checks
#   for them trying to do clever stateful IP stuff.
# - uh, we're also getting like 8500 cert-manager checks a day?  That seems high?
NOT_BOT_CHECK='where !contains(user_agent, "bot") | where !contains(user_agent, "cert-manager")'
NOT_BOT_CHECK+=' | where !contains(user_agent, "Bot") | where !contains(user_agent, "LinkChecker")'
NOT_BOT_CHECK+=' | where user_agent != "The Knowledge AI" | where !contains(user_agent, "Riddler")'
NOT_BOT_CHECK+=' | where !contains(user_agent, "search") | where !contains(user_agent, "spider")'
# Something using a focus user agent keeps asking for amazon and wikipedia icons
FOCUS_CHECK='where contains(user_agent, "Firefox%20Focus")'
NOT_FOCUS_CHECK='where !contains(user_agent, "Firefox%20Focus")'

# Requests that aren't against our host (ex: random IP) or are a POST are sketchy.
# We just ignore these specific requests but I guess we could try and statefully
# ignore the IP.
SKETCHY_CHECK='where req_host == "searchfox.org" && req_method == "GET"'

# ## Parse the Path and Validate Tree Names ##
PARSE_PATH='parse regex "(?P<sf_tree>[^/]+)/(?P<sf_endpoint>[^/\?]+)(?P<_ign_path>/?(?P<sf_path>[^\?]*))(?P<_ign_query>\??(?P<sf_query>.*))" from req_path nodrop'
# This successfully parses a number of things where the tree is not actually a
# real tree and instead an attempt for an attacker to find a vulnerable wordpress
# instance, so let's filter down trees to those we know exist or might exist in
# the future.  (Like, matching "mozilla-" or "comm-" is a sufficient constraint
# for mozilla-central and beta and release and our ESRs).
PARSE_PATH+=' | where isEmpty(sf_tree) || (contains(sf_tree, "mozilla-") || contains(sf_tree, "comm-")'
PARSE_PATH+=' || sf_tree == "wubkat" || sf_tree == "kaios" || sf_tree == "glean"'
PARSE_PATH+=' || sf_tree == "nss" || sf_tree == "whatwg-html" || sf_tree == "ecma262"'
PARSE_PATH+=' || sf_tree == "l10n" || sf_tree == "llvm" || sf_tree == "rust"'
PARSE_PATH+=' || sf_tree == "mingw" || sf_tree == "mingw_moz"'
PARSE_PATH+=')'
# We're also getting a bunch of gibberish endpoints, so let's constrain those too.
PARSE_PATH+=' | where isEmpty(sf_endpoint)'
PARSE_PATH+=' || sf_endpoint == "commit" || sf_endpoint == "commit-info"'
PARSE_PATH+=' || sf_endpoint == "complete" || sf_endpoint == "define"'
PARSE_PATH+=' || sf_endpoint == "diff" || sf_endpoint == "file-lists"'
PARSE_PATH+=' || sf_endpoint == "hgrev" || sf_endpoint == "pages"'
PARSE_PATH+=' || sf_endpoint == "query" || sf_endpoint == "raw-analysis"'
PARSE_PATH+=' || sf_endpoint == "rev" || sf_endpoint == "search"'
PARSE_PATH+=' || sf_endpoint == "sorch" || sf_endpoint == "source"'
PARSE_PATH+=' || sf_endpoint == "static"'


# ## Categorize ##
#
# Attempt to categorize the request; we start from the sf_endpoint but do some
# overrides when that's not enough.

# favicon is favicon
CATEGORIZE='if(req_path == "favicon.ico", "favicon", sf_endpoint) as category'
# apple-touch-icon stuff is favicon
CATEGORIZE+=' | if(isEmpty(sf_tree) && contains(req_path, "apple-touch-icon"), "favicon", category) as category'
# just loading the root page should be specially noted
CATEGORIZE+=' | if(isEmpty(req_path), "root", category) as category'
# Let's also capture people having fashioned "mozilla-central" or "mozilla-central/"
# URLs that don't work.
CATEGORIZE+=' | if(req_path == "mozilla-central" || req_path == "mozilla-central/", "bare-tree", category) as category'
# Let's introduce a synthetic category for "search.js" as a representative single
# static resource so that we can reason about the implication of cache validation
# without having to know how many different static resources we have in that set.
#
# Right now our validity is 2 minutes.
CATEGORIZE+=' | if(sf_path == "js/search.js", "static1", category) as category'
# lastly, let's discard entries that have a null category.  In general these are
# sketchy requests that are looking for vulnerable server.
CATEGORIZE+=' | where !isEmpty(category)'

IP_GROUPING='count as req_count by sf_tree, category, client_ip'
IP_GROUPING+=' | sum(req_count) as total_reqs, count as ips_1rq'
IP_GROUPING+=', count(req_count >= 2) as ips_2rq'
IP_GROUPING+=', sum(if(req_count >= 2, req_count, 0)) as reqs_2rq'
IP_GROUPING+=', count(req_count >= 4) as ips_4rq'
IP_GROUPING+=', sum(if(req_count >= 4, req_count, 0)) as reqs_4rq'
IP_GROUPING+=', count(req_count >= 8) as ips_8rq'
IP_GROUPING+=', sum(if(req_count >= 8, req_count, 0)) as reqs_8rq'
IP_GROUPING+=', count(req_count >= 16) as ips_16rq'
IP_GROUPING+=', sum(if(req_count >= 16, req_count, 0)) as reqs_16rq'
IP_GROUPING+=', count(req_count >= 32) as ips_32rq'
IP_GROUPING+=', sum(if(req_count >= 32, req_count, 0)) as reqs_32rq'
IP_GROUPING+=', count(req_count >= 64) as ips_64rq'
IP_GROUPING+=', sum(if(req_count >= 64, req_count, 0)) as reqs_64rq'
IP_GROUPING+=', count(req_count >= 128) as ips_128rq'
IP_GROUPING+=', sum(if(req_count >= 128, req_count, 0)) as reqs_128rq'
IP_GROUPING+=', count(req_count >= 256) as ips_256rq'
IP_GROUPING+=', sum(if(req_count >= 256, req_count, 0)) as reqs_256rq'
IP_GROUPING+=', count(req_count >= 512) as ips_512rq'
IP_GROUPING+=', sum(if(req_count >= 512, req_count, 0)) as reqs_512rq'
IP_GROUPING+=', count(req_count >= 1024) as ips_1024rq'
IP_GROUPING+=', sum(if(req_count >= 1024, req_count, 0)) as reqs_1024rq'
IP_GROUPING+=', count(req_count >= 2048) as ips_2048rq'
IP_GROUPING+=', sum(if(req_count >= 2048, req_count, 0)) as reqs_2048rq'
IP_GROUPING+=' by sf_tree, category | sort by sf_tree, category'

# From stackoverflow answer https://stackoverflow.com/a/34282594/17236969 by
# "peak" and edited by "TWiStErRob", a JQ pipeline to convert our JSON
# to a CSV rep that we can upload to Google sheets.  Note that this does depend
# on having a version of angle-grinder with https://github.com/rcoh/angle-grinder/pull/177
# or successor in it in order to have the keys ordered as desired, although this
# conversion is clever enough that we could also just insert a synthetic first
# row.
JQ_CSV_SCRIPT='(.[0] | keys_unsorted) as $firstkeys'
JQ_CSV_SCRIPT+=' | (map(keys) | add | unique) as $allkeys'
JQ_CSV_SCRIPT+=' | ($firstkeys + ($allkeys - $firstkeys)) as $cols'
JQ_CSV_SCRIPT+=' | ($cols, (.[] as $row | $cols | map($row[.])))'
JQ_CSV_SCRIPT+=' | @csv'

AGRIND_CORE="* | ${PARSE_EXPR} | ${NOT_BOT_CHECK} | ${NOT_FOCUS_CHECK} | ${SKETCHY_CHECK} | ${PARSE_PATH} | ${CATEGORIZE}"
#echo agrind "${AGRIND_CMD} | fields req_host, req_method"
if [[ ${1:-} == "export" ]]; then
  cat *.log | agrind -o json "${AGRIND_CORE} | ${IP_GROUPING}" | jq -r "${JQ_CSV_SCRIPT}"
else
  cat *.log | agrind  "${AGRIND_CORE} | ${IP_GROUPING}"

#cat *.log | agrind  "${AGRIND_CORE} | where sf_endpoint == \"static\""
#cat *.log | agrind "${AGRIND_CORE} | where sf_tree == \"mozilla-central\" && category == \"search\" | count by sf_tree, category, client_ip"

#cat *.log | agrind "${AGRIND_CORE} | where category == \"favicon\" | count by client_ip"
#cat *.log | agrind "${AGRIND_CORE} | fields category, req_path, sf_tree, sf_endpoint"
#cat *.log | agrind "${AGRIND_CORE} | where req_path == \"robots.txt\" | fields user_agent"

#cat *.log | agrind "* | ${PARSE_EXPR} | fields request"
#cat *.log | agrind "* | ${PARSE_EXPR} | ${NOT_BOT_CHECK} | ${NOT_FOCUS_CHECK} | count by client_ip, user_agent"
#cat *.log | agrind "* | ${PARSE_EXPR} | ${NOT_BOT_CHECK} | ${FOCUS_CHECK} | count"
#

fi
