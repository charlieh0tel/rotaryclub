BEGIN {
  ok = 1
}
NR == 1 {
  next
}
{
  mode = $1
  scenario = $2
  detection = $7 + 0.0
  false_pos = $8 + 0.0
  mean_err = $9 + 0.0
  p95_err = $10 + 0.0

  min_det = 0.0
  max_fp = 1.0
  max_mean = 999.0
  max_p95 = 999.0

  if (scenario == "clean") {
    min_det = 0.95
    max_fp = 0.05
    max_mean = 1.0
    max_p95 = 2.0
  } else if (scenario == "noisy_jittered") {
    min_det = 0.90
    max_fp = 0.08
    max_mean = 1.3
    max_p95 = 2.5
  } else if (scenario == "dropout_burst") {
    min_det = 0.88
    max_fp = 0.10
    max_mean = 1.4
    max_p95 = 2.6
  } else if (scenario == "impulsive_interference") {
    if (mode == "simple") {
      min_det = 0.30
    } else {
      min_det = 0.85
    }
    max_fp = 0.15
    max_mean = 1.5
    max_p95 = 2.8
  } else {
    printf("FAIL unknown scenario row: %s\n", $0)
    ok = 0
    next
  }

  if (mode != "dpll" && mode != "simple") {
    printf("FAIL unknown mode row: %s\n", $0)
    ok = 0
    next
  }

  if (detection < min_det || false_pos > max_fp || mean_err > max_mean || p95_err > max_p95) {
    printf("FAIL row: %s (det=%.6f fp=%.6f mean=%.6f p95=%.6f; limits det>=%.2f fp<=%.2f mean<=%.2f p95<=%.2f)\n",
      $0, detection, false_pos, mean_err, p95_err, min_det, max_fp, max_mean, max_p95)
    ok = 0
  }
}
END {
  if (!ok) {
    exit 1
  }
}
