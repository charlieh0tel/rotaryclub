#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${1:-target/timing-metrics}"
OUT_CSV="${OUT_DIR}/north_tick_timing_metrics.csv"
mkdir -p "${OUT_DIR}"

echo "Running north tick timing metrics example..."
cargo run --release --example north_tick_timing_metrics > "${OUT_CSV}"
echo "Wrote ${OUT_CSV}"

awk -F, -f scripts/north_tick_timing_metrics_thresholds.awk "${OUT_CSV}"

echo "North tick timing metrics thresholds: PASS"
