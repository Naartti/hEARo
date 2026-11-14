#!/usr/bin/env bash
# Generate the Xcode project into a temp dir to prove app/project.yml is valid.
. "$(dirname "$0")/lib.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
xcodegen generate --quiet --spec app/project.yml --project "$tmp" --project-root app
test -d "$tmp/hEARo.xcodeproj"
