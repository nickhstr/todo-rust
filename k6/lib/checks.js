// Shared assertion helpers. Each scenario imports these instead of
// calling `check()` directly, so endpoint tagging stays consistent and
// the Grafana dashboard's per-endpoint panel finds the labels it wants.

import { check } from 'k6';

// Endpoint names — kept as a const so a typo at the call site doesn't
// silently create a new metric label.
export const Endpoint = {
  Healthz: 'healthz',
  Readyz: 'readyz',
  Index: 'index',
  Login: 'login',
  Signup: 'signup',
  TodoCreate: 'todo-create',
  TodoToggle: 'todo-toggle',
  TodoDelete: 'todo-delete',
  StaticCss: 'static-css',
  Metrics: 'metrics',
};

// Convenience: build a `params` object with the endpoint tag baked in.
//
//   http.get(url, params(Endpoint.Index, { redirects: 0 }))
//
export function params(endpoint, extra = {}) {
  return { ...extra, tags: { endpoint, ...(extra.tags || {}) } };
}

export function assertStatus(res, expected, label) {
  return check(res, {
    [`${label} status==${expected}`]: (r) => r.status === expected,
  });
}

export function assertBodyContains(res, needle, label) {
  return check(res, {
    [`${label} body contains "${needle}"`]: (r) =>
      typeof r.body === 'string' && r.body.includes(needle),
  });
}

export function assertHeader(res, header, expected, label) {
  return check(res, {
    [`${label} header ${header}==${expected}`]: (r) =>
      (r.headers[header] || r.headers[header.toLowerCase()]) === expected,
  });
}
