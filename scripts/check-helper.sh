#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [[ $# -ne 4 ]]
then
    set +x  # Turn off echoing of commands and output only relevant things to avoid bloating logfiles
    echo "usage: $0 <check-disk> <server-url> <searchfox user exposed path> <canonical symbol name as found in data-symbols>"
    exit 1
fi

CHECK_DISK=$1
CHECK_SERVER_URL=$2
SEARCHFOX_PATH=$3
SYMBOL_NAME=$4

function fail {
  set +x  # Turn off echoing of commands and output only relevant things to avoid bloating logfiles
  echo -e "=========================================\n    FAILED INDEXING INTEGRITY CHECK\nError: $1\n========================================="
  exit "${2-1}"  ## Return a code specified by $2 or 1 by default.
}

if [[ $CHECK_DISK ]]
then
  # We now gzip these files as part of the indexing process prior to running
  # these checks.  Note that because we use the nginx try_files mechanism
  # probing for the non .gz versions we also need to create zero-length versions
  # of the files which defeats zgrep's magic failover to trying the .gz suffixed
  # version of the file.
  LOCAL_ANALYSIS_PATH=$INDEX_ROOT/analysis/$SEARCHFOX_PATH.gz
  LOCAL_HTML_PATH=$INDEX_ROOT/file/$SEARCHFOX_PATH.gz

  # The analysis file should exist.
  if [[ ! -f $LOCAL_ANALYSIS_PATH ]]; then
    # logic to run if the file didn't exist / wasn't a file
    set +x  # Turn off echoing of commands and output only relevant things to avoid bloating logfiles
    echo "The expected analysis file $LOCAL_ANALYSIS_PATH does not exist!"
    # TODO: Maybe do a secondary check here so that we can report when the
    # analysis file has ended up in a per-platform directory if the path started
    # with __GENERATED__.
    exit 1
  fi

  # GREP NOTE!
  # We use "-m1" to return only the first match for what we're looking for.
  # This is primarily done to avoid filling up the indexing log with unnecessary
  # spam, but it also helps performance.  It does also impact curl, see the curl
  # note below.

  # Look for a target record that contains the symbol name.  (Source records may
  # currently contain multiple symbol names, in which case we wouldn't want to
  # look for the closing quote or would need to allow for it to be also be a
  # comma.)
  zgrep -m1 "\"sym\":\"$SYMBOL_NAME\"" "$LOCAL_ANALYSIS_PATH" || fail "No symbol: $SYMBOL_NAME in analysis in $LOCAL_ANALYSIS_PATH"

  # The output file should exist.
  if [[ ! -f "$LOCAL_ANALYSIS_PATH" ]]; then
    # logic to run if the file didn't exist / wasn't a file
    set +x  # Turn off echoing of commands and output only relevant things to avoid bloating logfiles
    echo "The expected analysis file $LOCAL_ANALYSIS_PATH does not exist!"
    exit 1
  fi

  # The output file should explicitly reference the symbol name as part of
  # `data-symbols`.  We allow for a comma or closing quote.
  zegrep -m1 -e "data-symbols=\"$SYMBOL_NAME[\",]" "$LOCAL_HTML_PATH"  || fail "No symbol: $SYMBOL_NAME in HTML in $LOCAL_HTML_PATH"

  # Note: It would be neat to check the crossref database here, but the file
  # gets very large and it's much more efficient to just ask the webserver.
fi

if [[ $CHECK_SERVER_URL ]]
then
  SERVER_ANALYSIS_URL=${CHECK_SERVER_URL}${TREE_NAME}/raw-analysis/${SEARCHFOX_PATH}
  SERVER_HTML_URL=${CHECK_SERVER_URL}${TREE_NAME}/source/${SEARCHFOX_PATH}
  SERVER_SYMBOL_SEARCH_URL=${CHECK_SERVER_URL}${TREE_NAME}/search?q=symbol:${SYMBOL_NAME}

  # CURL NOTE!
  # When curl is piped or process substitution is used (which also creates a
  # pipe) and the reader of that pipe (grep/egrep/jq) closes the pipe early,
  # curl will by default report an error about failing to write to its buffer.
  #
  # We don't actually care about this, so we silence it via "-s".  And in
  # general we don't care if curl fails, as we're looking for positive presence
  # of specific output in the returned data.

  # Curl flags we intentionally use:
  # -f: Fail on server error codes rather than returning the error document.
  # -s: Silence the progress bar and error output.

  # Check the analysis file exists and contains the expected symbol using the
  # same check from the disk case.
  grep -m1 "\"sym\":\"$SYMBOL_NAME\"" <( curl -fs "$SERVER_ANALYSIS_URL" ) || fail "No symbol: $SYMBOL_NAME in analysis served at $SERVER_ANALYSIS_URL"

  # Check the HTML file exists and contains the expected symbol using the same
  # check from the disk case.
  egrep -m1 -e "data-symbols=\"$SYMBOL_NAME[\",]" <( curl -fs "$SERVER_HTML_URL" ) || fail "No symbol: $SYMBOL_NAME in HTML served at $SERVER_HTML_URL"

  # This JQ expression looks for a definition that has the searchfox path as its
  # path.  Explanation:
  # - to_entries converts the dictionary that looks like
  #   { normal, generated, tests } into [{key: normal, value: ...}]
  JQ_FIND_DEF_EXPR="to_entries"
  # - This returns the Definitions hitlist for the given group or an empty array
  #   if there was no set of Definitions.
  JQ_FIND_DEF_EXPR+=" | map(.value.Definitions? // [])"
  # - flatten merges all of the definitions hitlists together into one list.
  JQ_FIND_DEF_EXPR+=" | flatten"
  # - This produces a list of booleans indicating whether the given hitlist
  #   contained the given path.
  JQ_FIND_DEF_EXPR+=" | map(.path == \"$SEARCHFOX_PATH\")"
  # - any returns true if there were any true values.
  JQ_FIND_DEF_EXPR+=" | any"

  jq "$JQ_FIND_DEF_EXPR" <( curl -fs -H "Accept: application/json" "$SERVER_SYMBOL_SEARCH_URL" )  || fail "No symbol: $SYMBOL_NAME in search results from $SERVER_SYMBOL_SEARCH_URL"
fi

set +x  # Turn off echoing of commands and output only relevant things to avoid bloating logfiles
echo -e "=========================================\nSUCCESS: Integrity check passed for $SYMBOL_NAME in $SEARCHFOX_PATH\n========================================="
