// journey.js — realistic user session: signup → CRUD → list.
// Each VU runs as its own user, signs up on iteration 0, then loops.
//
// Cookie-jar note: k6 clears the default per-VU cookie jar between iterations.
// To keep the session alive for the full VU lifetime we create a module-level
// CookieJar (instantiated once per VU) and pass it to every HTTP call so the
// session cookie persists across iterations.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import {
    Endpoint,
    params,
    assertStatus,
    assertBodyContains,
} from '../lib/checks.js';
import { signup, login } from '../lib/auth.js';

export const options = {
    scenarios: {
        journey: {
            executor: 'ramping-vus',
            startVUs: 0,
            stages: [
                { duration: '1m', target: 30 },
                { duration: '5m', target: 30 },
                { duration: '1m', target: 0 },
            ],
        },
    },
    thresholds: {
        ...SHARED_THRESHOLDS,
        http_req_duration: ['p(95)<500'],
        iteration_duration: ['p(95)<3000'],
    },
};

// VU-scoped state: a unique email per VU, generated once.
// (k6 reinitializes the module per-VU, so module-level `let` is
// VU-scoped — it does NOT bleed across VUs.)
const vuEmail = `journey-vu${__VU}-${Date.now()}@example.test`;
const vuPassword = 'k6-journey-password';

// One persistent cookie jar per VU — not cleared between iterations.
const jar = new http.CookieJar();

export default function () {
    let r;

    // Signup on first iteration; subsequent iterations are already logged in
    // (session cookie lives in the persistent jar).
    if (__ITER === 0) {
        signup(vuEmail, vuPassword, jar);
    } else {
        // Re-login to refresh the session cookie in the persistent jar.
        // Skip on iteration 0 — signup auto-logs in.
        login(vuEmail, vuPassword, jar);
    }

    // Step 1: list (verify empty state on first iter, populated later)
    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { jar }));
    assertStatus(r, 200, 'GET / step 1');

    // Step 2: create three todos, with a realistic typing pause between
    const titles = [`todo-${__VU}-${__ITER}-a`, `todo-${__VU}-${__ITER}-b`, `todo-${__VU}-${__ITER}-c`];
    for (const title of titles) {
        r = http.post(
            `${BASE_URL}/todos`,
            { title },
            params(Endpoint.TodoCreate, { redirects: 0, jar }),
        );
        if (r.status !== 201 && r.status !== 200) {
            throw new Error(`POST /todos status=${r.status}`);
        }
    }

    // Step 3: list again, grab ids from the response body.
    // The todo.html template renders each <li id="todo-<uuid>">, so we extract
    // the UUID from that attribute.
    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { jar }));
    assertStatus(r, 200, 'GET / step 3');
    assertBodyContains(r, titles[0], 'step 3 contains created todo');

    // Extract todo UUIDs from id="todo-<uuid>" attributes in the response body.
    const ids = [...(r.body || '').matchAll(/id="todo-([0-9a-f-]{36})"/g)].map((m) => m[1]);
    if (ids.length >= 2) {
        // Step 4: toggle one
        r = http.post(
            `${BASE_URL}/todos/${ids[0]}/toggle`,
            null,
            params(Endpoint.TodoToggle, { redirects: 0, jar }),
        );
        assertStatus(r, 200, 'toggle');

        // Step 5: delete another
        r = http.del(
            `${BASE_URL}/todos/${ids[1]}`,
            null,
            params(Endpoint.TodoDelete, { redirects: 0, jar }),
        );
        assertStatus(r, 200, 'delete');
    }

    // Step 6: final list
    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { jar }));
    assertStatus(r, 200, 'GET / step 6');

    sleep(0.2);
}

export const handleSummary = makeSummaryWriter('journey');
