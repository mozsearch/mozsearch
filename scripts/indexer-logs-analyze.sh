#!/usr/bin/env bash

SCRIPT_DIR=$(dirname $0)

grep "^Performing .* step for" index-* \
  | ${SCRIPT_DIR}/indexer-logs-print.py
