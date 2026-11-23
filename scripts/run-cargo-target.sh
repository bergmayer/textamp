#!/bin/sh
set -eu
case "$1" in
    */deps/*)
        test_profile=$(mktemp -d "${TMPDIR:-/tmp}/textamp-test.XXXXXXXX")
        trap 'rm -rf "$test_profile"' 0
        mkdir -p "$test_profile/config" "$test_profile/data" "$test_profile/cache" "$test_profile/state"
        XDG_CONFIG_HOME="$test_profile/config" \
        XDG_DATA_HOME="$test_profile/data" \
        XDG_CACHE_HOME="$test_profile/cache" \
        XDG_STATE_HOME="$test_profile/state" \
        TEXTAMP_CARGO_TEST_PROFILE="$test_profile" "$@"
        ;;
    *) exec "$@" ;;
esac
