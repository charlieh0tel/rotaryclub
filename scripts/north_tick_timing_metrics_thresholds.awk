BEGIN {
  ok = 1
}
NR == 1 {
  next
}
{
  scenario = $1
  detection = $6 + 0.0
  mean_err = $7 + 0.0
  p95_err = $8 + 0.0

  if (scenario == "clean") {
    if (detection < 0.95 || mean_err > 1.0 || p95_err > 2.0) {
      printf("FAIL clean row: %s (det=%.6f mean=%.6f p95=%.6f)\n", $0, detection, mean_err, p95_err)
      ok = 0
    }
  } else if (scenario == "noisy_jittered") {
    if (detection < 0.90 || mean_err > 1.3 || p95_err > 2.5) {
      printf("FAIL noisy_jittered row: %s (det=%.6f mean=%.6f p95=%.6f)\n", $0, detection, mean_err, p95_err)
      ok = 0
    }
  } else {
    printf("FAIL unknown scenario row: %s\n", $0)
    ok = 0
  }
}
END {
  if (!ok) {
    exit 1
  }
}
