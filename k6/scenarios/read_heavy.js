// read_heavy.js — ramps to 1000 req/s on GET / with cache warm.
//
// setup() bulk-seeds users + todos via the API (idempotent).
// Each VU is assigned one seed user, logs in on iteration 0, then loops.
//
// Cookie-jar note: k6 clears the default per-VU cookie jar between iterations.
// To keep the session alive for the full VU lifetime we create a module-level
// CookieJar (instantiated once per VU) and pass it to every HTTP call so the
// session cookie persists across iterations.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import { Endpoint, params, assertStatus } from '../lib/checks.js';
import { seedUsersWithTodos } from '../lib/seed.js';
import { login } from '../lib/auth.js';

const USERS = parseInt(__ENV.USERS || '50', 10);
const TODOS_PER_USER = parseInt(__ENV.TODOS_PER_USER || '10', 10);

// One persistent cookie jar per VU — not cleared between iterations.
const jar = new http.CookieJar();

export const options = {
  setupTimeout: '5m',
  scenarios: {
    read_heavy: {
      executor: 'ramping-arrival-rate',
      startRate: 50,
      timeUnit: '1s',
      preAllocatedVUs: 50,
      maxVUs: 200,
      stages: [
        { duration: '1m', target: 100 },
        { duration: '2m', target: 500 },
        { duration: '2m', target: 1000 },
        { duration: '2m', target: 1000 },
        { duration: '1m', target: 0 },
      ],
    },
  },
  thresholds: {
    ...SHARED_THRESHOLDS,
    'http_req_duration{endpoint:index}': ['p(95)<500'],
  },
};

export function setup() {
  return { users: seedUsersWithTodos(USERS, TODOS_PER_USER) };
}

export default function (data) {
  // On the first iteration each VU logs in as its assigned user and stores
  // the session cookie in the persistent jar for subsequent iterations.
  if (__ITER === 0) {
    const user = data.users[(__VU - 1) % data.users.length];
    login(user.email, user.password, jar);
  }

  const roll = Math.random();
  let r;
  if (roll < 0.9) {
    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { redirects: 0, jar }));
    assertStatus(r, 200, 'GET /');
  } else if (roll < 0.95) {
    r = http.get(`${BASE_URL}/login`, params(Endpoint.Login, { jar }));
    assertStatus(r, 200, 'GET /login');
  } else {
    r = http.get(`${BASE_URL}/healthz`, params(Endpoint.Healthz, { jar }));
    assertStatus(r, 200, 'healthz');
  }
}

export const handleSummary = makeSummaryWriter('read_heavy');
