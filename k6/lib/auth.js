// Auth helpers — signup + login. Each VU gets its own automatic cookie
// jar, so subsequent requests in the same iteration use the session
// transparently.

import http from 'k6/http';
import { BASE_URL } from './config.js';
import { params, Endpoint } from './checks.js';

// POST /signup. Returns the response. Treats 409 (user already exists)
// as success for idempotent seeding.
// Pass a `jar` (http.CookieJar instance) to persist the session cookie
// across k6 iterations — the default per-VU cookie jar is cleared between
// iterations, so long-lived VU sessions need an explicit persistent jar.
export function signup(email, password, jar = null) {
  const extra = jar ? { redirects: 0, jar } : { redirects: 0 };
  const res = http.post(
    `${BASE_URL}/signup`,
    { email, password },
    params(Endpoint.Signup, extra),
  );
  if (res.status !== 303 && res.status !== 409) {
    throw new Error(
      `signup ${email} unexpected status ${res.status} body=${(res.body || '').slice(0, 200)}`,
    );
  }
  return res;
}

// POST /login. Returns the response. Throws on anything other than 303.
// Pass a `jar` (http.CookieJar instance) to persist the session cookie
// across k6 iterations — the default per-VU cookie jar is cleared between
// iterations, so long-lived VU sessions need an explicit persistent jar.
export function login(email, password, jar = null) {
  const extra = jar ? { redirects: 0, jar } : { redirects: 0 };
  const res = http.post(
    `${BASE_URL}/login`,
    { email, password },
    params(Endpoint.Login, extra),
  );
  if (res.status !== 303) {
    throw new Error(
      `login ${email} unexpected status ${res.status} body=${(res.body || '').slice(0, 200)}`,
    );
  }
  return res;
}
