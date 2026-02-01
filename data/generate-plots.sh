#!/bin/bash

set -o errexit
set -o nounset


PLOT_ARGS="--min-confidence 0.1 --min-coherence 0.1"

for f in ../data/*.wav; do
  name=$(basename "$f" .wav)
  echo "=== Processing $name ==="
  cargo run --release -- -i "$f" -f csv 2>"${name}.log" \
    | python3 ../scripts/plot_bearings.py $PLOT_ARGS
  mv /tmp/bearings_plot.png "${name}.png"
done
