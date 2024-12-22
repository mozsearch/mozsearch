#!/usr/bin/env bash

# This script locates all .scip files in the OBJDIR and then generates analysis
# data from the scip indices.
#
# This data-flow path is evolved from the WIP rust SCIP analysis functionality,
# and changes are still in flight.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -lt 2 ]
then
    echo "Usage: scip-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# Extract the list of scip_subtrees for our tree of interest.  We extract each
# full POJO object rather than its keys in this pass because we explicitly allow
# for the empty string as a value but the `read` command does not like that.
# This also arguably allows for some extra flexibility when processing inside
# the loop, which is part of why we moved to this.  (Another option, previously
# used is that we could avoid passing `-r` which would leave the strings quoted,
# but then re-interpreting them inside the loop is a hassle and the cleanest
# option was invoking jq again with `-r`, but then we might as well be picking
# out of the object.)
#
# Note that another option would be to just have the scip-indexer directly
# access the information from the config file.  We're not doing that in order to
# faciliate use-cases like mozilla-central's per-platform in
# process-tc-artifacts.sh.  Note that for these per-platform cases we expect
# those scripts to just directly invoke `scip-indexer` themselves.
SCIP_SUBTREE_INFOS=$(jq -Mc ".trees[\"${TREE_NAME}\"].scip_subtrees | to_entries? | .[]?" ${CONFIG_FILE})

# Note: This structuring avoids use of a pipe and sub-shells which allows us to
# mutate global variables if we want.
if [[ $SCIP_SUBTREE_INFOS ]]; then
  while read -r subtree_obj; do
    scip_tree_name=$(jq -Mr '.key' <<< "$subtree_obj")
    scip_index_path=$(jq -Mr '.value.scip_index_path' <<< "$subtree_obj")
    subtree_root=$(jq -Mr '.value.subtree_root' <<< "$subtree_obj")
    scip-indexer \
      "$CONFIG_FILE" \
      "$TREE_NAME" \
      --subtree-name "${scip_tree_name}" \
      --subtree-root "${subtree_root}" \
      "${scip_index_path}"
  done <<< "$SCIP_SUBTREE_INFOS"
fi
