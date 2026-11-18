#!/bin/sh
# macOS: double-click to start agenttable from this folder and open the browser.
cd "$(dirname "$0")" || exit 1
exec ./agenttable --open "$@"
