// smoke.js — 30s sanity check that exercises unauth endpoints + the
// static asset pipeline. Run before every other scenario.

import http from 'k6/http';
import { sleep } from 'k6';
import { BASE_URL, SHARED_THRESHOLDS, makeSummaryWriter } from '../lib/config.js';
import {
    Endpoint,
    params,
    assertStatus,
    assertBodyContains,
} from '../lib/checks.js';

export const options = {
    scenarios: {
        smoke: {
            executor: 'constant-vus',
            vus: 1,
            duration: '30s',
        },
    },
    thresholds: {
        ...SHARED_THRESHOLDS,
        'http_req_duration{endpoint:healthz}': ['p(95)<20'],
    },
};

export default function () {
    let r;

    r = http.get(`${BASE_URL}/healthz`, params(Endpoint.Healthz));
    assertStatus(r, 200, 'healthz');

    r = http.get(`${BASE_URL}/readyz`, params(Endpoint.Readyz));
    assertStatus(r, 200, 'readyz');

    r = http.get(`${BASE_URL}/login`, params(Endpoint.Login));
    assertStatus(r, 200, 'GET /login');

    r = http.get(`${BASE_URL}/`, params(Endpoint.Index, { redirects: 0 }));
    assertStatus(r, 303, 'GET / unauth -> 303');

    r = http.get(`${BASE_URL}/static/css/app.css`, params(Endpoint.StaticCss));
    assertStatus(r, 200, 'static css');

    r = http.get(`${BASE_URL}/metrics`, params(Endpoint.Metrics));
    assertStatus(r, 200, 'metrics');
    assertBodyContains(r, 'http_requests_total', 'metrics body');

    sleep(0.2);
}

export const handleSummary = makeSummaryWriter('smoke');
