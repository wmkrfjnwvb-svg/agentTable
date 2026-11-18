#!/bin/sh
# Starts agenttable from this folder and opens the browser.
cd "$(dirname "$0")" || exit 1
exec ./agenttable --open "$@"
