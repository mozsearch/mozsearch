#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

OBJDIR="$1"

mkdir -p "$OBJDIR/__RUST__"

cp -r "$(rustc --print sysroot)/lib/rustlib/src/rust/src" "$OBJDIR/__RUST__"
