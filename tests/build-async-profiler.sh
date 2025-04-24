#!/bin/bash

set -exuo pipefail

# FIXME: this is a version of async-profiler that supports user JFR support
# When async-profiler make an actual versioned release (that will then have
# user JFR), cut over to that.

git clone https://github.com/async-profiler/async-profiler
cd async-profiler
git checkout 14c7e819b2054308da1bd980fe72137d2d0e0cf6
make -j16
