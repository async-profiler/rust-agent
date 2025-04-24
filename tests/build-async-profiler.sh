#!/bin/bash

set -exuo pipefail

git clone https://github.com/async-profiler/async-profiler
cd async-profiler
git checkout 14c7e819b2054308da1bd980fe72137d2d0e0cf6
make -j16
