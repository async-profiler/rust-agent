#!/bin/bash

# This test needs to following resources:
# 1. LD_LIBRARY_PATH set to an async-profiler with user JFR support
# 2. executable `./pollcatch-decoder` from `cd decoder && cargo build`
# 3. executable `./simple` from `RUSTFLAGS="--cfg tokio_unstable" cargo build --example simple`

set -exuo pipefail

dir="profiles"

mkdir -p $dir
rm -f $dir/*.jfr

# test the drop-path final report logic
rm -rf $dir/short
mkdir $dir/short
./simple --local $dir/short --duration 1s --reporting-interval 10s --no-clean-stop >$dir/short/log
cat $dir/short/log
grep "profiler task cancelled, attempting final report on drop" $dir/short/log
rm -rf $dir/short

# Pass --worker-threads 16 to make the test much less flaky since there is always some worker thread running
./simple --local $dir --duration 30s --reporting-interval 10s --worker-threads 16 --native-mem 4096

found_good=0

for profile in $dir/*.jfr; do
    duration=$(./pollcatch-decoder duration "$profile")
    # Ignore "partial" profiles of less than 8s
    if [[ $duration > 8 ]]; then
        found_good=1
    else
        echo "Profile $profile is too short"
        continue
    fi

    # Basic event presence check
    native_malloc_count=$(./pollcatch-decoder nativemem --type malloc "$profile" | wc -l)
    if [ "$native_malloc_count" -lt 1 ]; then
        echo "No native malloc events found in $profile"
        exit 1
    fi

    short_sleeps_100=$(./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us | ( grep -c short_sleep_2 || true ))
    short_sleeps_1000=$(./pollcatch-decoder longpolls --stack-depth=10 "$profile" 1000us | ( grep -c short_sleep_2 || true ))
    long_sleeps_100=$(./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us | ( grep -c accidentally_slow_2 || true ))
    long_sleeps_1000=$(./pollcatch-decoder longpolls --stack-depth=10 "$profile" 1000us | ( grep -c accidentally_slow_2 || true ))
    # Long sleeps should occur in both the 100us and 1000us filters
    if [ "$long_sleeps_100" -lt 1 ]; then
        echo "No long sleeps in 100us"
        ./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us
        exit 1
    fi
    if [ "$long_sleeps_1000" -lt 1 ]; then
        echo "No long sleeps in 1000us"
        ./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us
        exit 1
    fi
    # short sleeps should only occur in the 100us filter. Allow a small number of short sleeps if there was OS jank
    if [ "$short_sleeps_100" -lt 5 ]; then
        echo "No short sleeps in 100us"
        ./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us
        exit 1
    fi
    if [ "$short_sleeps_1000" -gt 5 ]; then
        echo "Too many short sleeps in 1000us"
        ./pollcatch-decoder longpolls --stack-depth=10 "$profile" 100us
        exit 1
    fi
done

if [ "$found_good" -eq 0 ]; then
    echo Found no good profiles
    exit 1
fi
