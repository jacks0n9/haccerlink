#!/usr/bin/env bash
# Build the libticalcs2 oracle. Run inside `nix develop .#oracle`.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p build
# -rdynamic exports our ticables_cable_send/recv so libticalcs binds to them.
gcc -O1 -g -Wall -rdynamic -o build/oracle oracle.c \
    $(pkg-config --cflags --libs ticalcs2 ticables2 tifiles2 ticonv glib-2.0)
echo "built $(pwd)/build/oracle"
