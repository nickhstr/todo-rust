// Shared k6 configuration: base URL, default thresholds, summary writer.
//
// BASE_URL defaults to the docker-internal app hostname; override via
// `-e BASE_URL=http://localhost:3000` when running k6 against a host
// binary instead of the in-compose service.

import { textSummary } from 'https://jslib.k6.io/k6-summary/0.0.2/index.js';

export const BASE_URL = __ENV.BASE_URL || 'http://app:3000';

export const SHARED_THRESHOLDS = {
  http_req_failed: ['rate<0.01'],
  checks: ['rate>0.99'],
};

// Build a `handleSummary` that writes the full k6 summary, plus a small
// meta block (git sha + scenario name + base URL), to a JSON file under
// /results/. Path is timestamped + sha-suffixed so concurrent or repeat
// runs don't overwrite each other.
export function makeSummaryWriter(scenario) {
  return function handleSummary(data) {
    const ts = new Date().toISOString().replace(/[:.]/g, '-');
    const gitSha = __ENV.GIT_SHA || 'unknown';
    const meta = {
      scenario,
      git_sha: gitSha,
      base_url: BASE_URL,
      finished_at: new Date().toISOString(),
      test_run_duration_ms: data.state ? data.state.testRunDurationMs : null,
    };
    return {
      stdout: textSummary(data, { indent: ' ', enableColors: true }),
      [`/results/summary-${scenario}-${ts}-${gitSha}.json`]:
        JSON.stringify({ meta, ...data }, null, 2),
    };
  };
}
