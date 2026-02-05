#!/bin/bash

set -o errexit
set -o nounset

PLOT_ARGS="--min-confidence 0.1 --min-coherence 0.1"

for f in ../data/*.wav; do
  name=$(basename "$f" .wav)
  echo "=== Processing $name ==="

  echo "  Running correlation method..."
  cargo run --release -- -i "$f" -m correlation -f csv 2>"${name}_corr.log" > "${name}_corr.csv"

  echo "  Running zero-crossing method..."
  cargo run --release -- -i "$f" -m zero-crossing -f csv 2>"${name}_zc.log" > "${name}_zc.csv"

  echo "  Generating plot..."
  python3 ../scripts/plot_bearings.py $PLOT_ARGS \
    --correlation "${name}_corr.csv" \
    --zero-crossing "${name}_zc.csv" \
    --output "${name}.png"

  rm -f "${name}_corr.csv" "${name}_zc.csv"
done
