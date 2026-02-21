#!/usr/bin/env bash

SCRIPT_DIR=$(dirname $0)

# See indexer-logs-print.py for the step log syntax.

grep -E "^Perform(ing|ed) .* (section|step) for" index-* \
  | ${SCRIPT_DIR}/indexer-logs-print.py
