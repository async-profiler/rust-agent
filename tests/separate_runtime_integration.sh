#!/bin/bash

# This test needs to following resources:
# 1. LD_LIBRARY_PATH set to an async-profiler with user JFR support
# 2. executable `./pollcatch-decoder` from `cd decoder && cargo build`
# 3. executable `./simple` from `RUSTFLAGS="--cfg tokio_unstable" cargo build --example simple`

set -exuo pipefail

dir="profiles-into-thread"

mkdir -p $dir
rm -f $dir/*.jfr

# Pass --worker-threads 16 to make the test much less flaky since there is always some worker thread running
./simple --local $dir --duration 15s --reporting-interval 3s --worker-threads 16 --native-mem 4096 --spawn-into-thread
found_good=0

for profile in $dir/*.jfr; do
    duration=$(./pollcatch-decoder duration "$profile")
    # Ignore "partial" profiles of less than 2s
    if [[ $duration > 2 ]]; then
        found_good=1
    else
        echo "Profile $profile is too short"
        continue
    fi

    # Basic event presence check
    # Intentionally not checking that profiling samples are correct, that requires
    # more samples which would slow things down. This is enough to make sure that
    # `spawn-into-thread` works.
    native_malloc_count=$(./pollcatch-decoder nativemem --type malloc "$profile" | wc -l)
    if [ "$native_malloc_count" -lt 1 ]; then
        echo "No native malloc events found in $profile"
        exit 1
    fi
done

if [ "$found_good" -eq 0 ]; then
    echo Found no good profiles
    exit 1
fi
