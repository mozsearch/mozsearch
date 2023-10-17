#!/usr/bin/env bash

# This is a script to determine when mozilla-central code coverage artifacts
# were uploaded.
#
# It uses the "taskcluster" client-shell binary tool from
# https://github.com/taskcluster/taskcluster/tree/main/clients/client-shell
# and the "coreapi" command line tool referenced by
# https://treeherder.mozilla.org/docs/ and installable via
# `pip install coreapi-cli` to do REST API stuff from inside bash.  It's
# probably least messy to install `coreapi-cli` with your searchfox venv active.
#
# Yeah, inside bash.
#
# I know.
#
# But it allows for iterative command line experimentation, I guess?  In any
# event, if this is ever needed on an ongoing basis this should absolutely be
# folded into something that is less excitingly cobbled together.

#set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

export TASKCLUSTER_ROOT_URL=https://firefox-ci-tc.services.mozilla.com

## Ask Treeherder for the more recent searchfox jobs
# Get the treeherder schema / metadata into place:
coreapi get https://treeherder.mozilla.org/docs/ > /dev/null

export SOURCE_REPO=https://hg.mozilla.org/mozilla-central

# Get searchfox jobs for the last N days and find their revision
echo "### Searchfox rev artifact creator times"
typeset -A SF_REVS_PRESENT
typeset -A SF_REV_COMPLETED

# Note: This will return jobs from trees other than mozilla-central right now...
# so we filter it out in the loop by looking up the task def... but it seems
# like we really should be able to filter this here, but it's not jumping out at
# me in the treeherder API.  (Maybe an integer id needs to be looked up?)
#
# Previously used for date: $(date -d "-7 days" -Iseconds -u)
# changed because of 500 errors that went away by removing the TZ.

SF_JOBS=$(coreapi action jobs list -p start_time__gt=$(date -d "-7 days" -u +"%FT%T") -p job_type_name=searchfox-linux64-searchfox/debug)
SF_JOB_REVS=$(jq -M -r '(.job_property_names | index("push_revision")) as $idx_rev | (.job_property_names | index("task_id")) as $idx_taskid | .results[] | .[$idx_rev], .[$idx_taskid]' <<< "$SF_JOBS")
ART_CSV=""
# In order to be able to manipulate the array, we need to ensure that there is
# no pipeline, because a pipeline will result in a sub-shell being created
# which cannot modify the root bash shell's variables!
while read -r sf_rev; read -r sf_taskid; do
  SF_REVS_PRESENT[${sf_rev}]=${sf_taskid}
  COMPLETED_SF=$(taskcluster api queue status $sf_taskid | jq -M -r '.status.runs[0].resolved')
  SF_DEF_INFO=$(taskcluster task def $sf_taskid)
  SF_SOURCE_REPO=$(jq -M -r '.payload.env.MOZ_SOURCE_REPO' <<< "$SF_DEF_INFO")
  
  # Filter out searchfox jobs that aren't from our desired repo.
  if [[ ${SF_SOURCE_REPO} != ${SOURCE_REPO} ]]; then
    continue
  fi
  
  SF_REV_COMPLETED[${sf_rev}]=${COMPLETED_SF}
  #echo "Set '${sf_rev}' to ${SF_REVS_PRESENT[${sf_rev}]}"
  ARTIFACT_NAME=project.relman.code-coverage.production.repo.mozilla-central.${sf_rev}
  
  echo "- ${sf_rev} artifact ${ARTIFACT_NAME}"
  echo "  - $COMPLETED_SF - indexing completed in job $sf_taskid"
  
  ARTIFACT_FINDTASK_INFO=$(taskcluster api index findTask ${ARTIFACT_NAME} 2>/dev/null || true)
  # The artifact may not exist.
  if [[ ${ARTIFACT_FINDTASK_INFO} == "" ]]; then
    echo "  - No such artifact!"
    continue
  fi
  ARTIFACT_TASKID=$(jq -M -r '.taskId' <<< "$ARTIFACT_FINDTASK_INFO")
  # DISCLAIMER: We're hard-coding run id of 0 here, which maybe is wrong in the face of infrastructure restarts.
  # we could totally find out the right run id from "queue status" and picking the last or successful one.
  ARTIFACT_INFO=$(taskcluster api queue listLatestArtifacts $ARTIFACT_TASKID)
  # I am assuming that the coverage artifact gets exactly 2 weeks from the date of uploading.
  # https://github.com/mozilla/code-coverage/blob/de635654bd3eae18c53c52c0010a42f362a04479/bot/code_coverage_bot/hooks/repo.py#L129 tells us that's right at the current time.
  ARTIFACT_UPLOAD_DATE=$(jq -M -r '.artifacts[] | select(.name ==  "public/code-coverage-report.json") | .expires | sub("\\.[0-9]+Z$"; "Z")  | fromdate - (14 * 24 * 60 * 60) | todate' <<< "$ARTIFACT_INFO")
  echo "  - ${ARTIFACT_UPLOAD_DATE} - artifact uploaded from task $ARTIFACT_TASKID"
  QUEUE_INFO=$(taskcluster api queue status $ARTIFACT_TASKID)
  QUEUE_STATUS=$(jq -Mc '.status.runs[] | { state, reasonResolved, started, resolved }' <<< $QUEUE_INFO)
  echo "  - artifact job statuses: $QUEUE_STATUS"
done <<< "$SF_JOB_REVS"


