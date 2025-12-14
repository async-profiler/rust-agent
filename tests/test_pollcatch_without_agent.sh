#!/bin/bash

# This test needs to following resources:
# 1. LD_LIBRARY_PATH set to an async-profiler with user JFR support
# 2. executable `./pollcatch-decoder` from `cd decoder && cargo build`
# 3. executable `./pollcatch-without-agent` from `cargo build --example pollcatch-without-agent`

set -exuo pipefail

dir="pollcatch-without-agent-jfr"

mkdir -p $dir
rm -f $dir/*.jfr

# Test that the pollcatch functions work fine with async-profiler in non-agent mode (test LD_PRELOAD mode)
LD_PRELOAD=libasyncProfiler.so ASPROF_COMMAND=start,event=cpu,jfr,file=$dir/output.jfr ./pollcatch-without-agent
./pollcatch-decoder longpolls --include-non-pollcatch $dir/output.jfr > $dir/output.txt
cat $dir/output.txt
grep -q 'poll of' $dir/output.txt
